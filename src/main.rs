use serde::{Deserialize, Serialize};
use chrono::{DateTime, Utc, Local, TimeZone, NaiveTime, Duration};

mod awair {
    use serde::{Deserialize, Serialize};
    use curl::easy::{Easy, List};

    #[derive(Debug, Deserialize, Serialize)]
    struct SensorData {
        comp: String,
        value: f64,
    }

    #[derive(Debug, Deserialize, Serialize)]
    struct Record {
        timestamp: String,
        sensors: Vec<SensorData>,
    }

    #[derive(Debug, Deserialize, Serialize)]
    pub struct Data {
        data: Vec<Record>,
    }

    fn get_temp(sv: &Vec<SensorData>) -> f64 {
        for s in sv.iter() {
            if s.comp.to_lowercase() == "temp" {
                return s.value;
            }
        }
        panic!("temp not found");
    }

    pub fn average_temp(data: &Data) -> f64 {
        let mut sum = 0.0;
        for r in data.data.iter() {
            sum += get_temp(&r.sensors);
        }
        return sum / (data.data.len() as f64);
    }

    pub fn read_average_temp(devtype: &str, devid: u64, token: &str) -> Result<f64, curl::Error> {
        let mut handle = Easy::new();
        let mut buf = Vec::new();
        let url = format!("https://developer-apis.awair.is/v1/users/self/devices/{}/{}/air-data/15-min-avg?limit=4", devtype, devid);
        handle.url(&url).unwrap();
        let mut list = List::new();
        let auth_hdr = format!("authorization: Bearer {}", token);
        list.append(&auth_hdr).unwrap();
        handle.http_headers(list).unwrap();
    
        let mut transfer = handle.transfer();
        transfer.write_function(|data| {
            buf.extend_from_slice(data);
            Ok(data.len())
        }).unwrap();
        transfer.perform().unwrap();
        drop(transfer);
    
        let data: Data = serde_json::from_slice(&buf[..]).unwrap();
    
        return Ok(average_temp(&data));
    }
}

mod daikin {
    use serde::{Deserialize, Serialize};
    use curl::easy::{Easy, List};
    
    pub struct SkyPort {
        email: String,
        access_token: String,
        refresh_token: String,
        device_id: String,
        device_data: DeviceData,
    }

    #[derive(Debug, Deserialize, Serialize)]
    struct LoginResult {
        accessToken: String,
        accessTokenExpiresIn: u64,
        refreshToken: Option<String>,
        tokenType: String,
    }

    #[derive(Debug, Deserialize, Serialize)]
    struct DeviceEntry {
        id: String,
        name: String,
    }

    #[derive(Debug, Deserialize, Serialize, Default)]
    struct DeviceData {
        #[serde(rename = "cspHome")]
        csp_home: f64,
        #[serde(rename = "hspHome")]
        hsp_home: f64,
        #[serde(rename = "tempIndoor")]
        temp_indoor: f64,
    }

    #[derive(Debug, Deserialize, Serialize)]
    struct APIError {
        message: String,
    }

    #[derive(Debug)]
    pub enum Error {
        HTTPError(curl::Error),
        APIError(u32, String),
    }

    fn access_webapi(url: &str, token: Option<&String>, body: Option<&String>) -> Result<(u32, Vec<u8>), curl::Error> {
        let mut handle = Easy::new();
        let mut buf: Vec<u8> = Vec::new();
        handle.url(url).unwrap();
        let mut list = List::new();
        list.append("Accept: application/json").unwrap();
        list.append("Content-Type: application/json").unwrap();
        if let Some(token) = token {
            let auth = format!("Authorization: Bearer {}", token);
            list.append(&auth).unwrap();
        }
        handle.http_headers(list).unwrap();

        if let Some(body) = body {
            handle.post(true).unwrap();
            handle.post_fields_copy(body.clone().into_bytes().as_slice()).unwrap();
        }

        let mut transfer = handle.transfer();
        transfer.write_function(|data| {
            buf.extend_from_slice(data);
            Ok(data.len())
        }).unwrap();
        transfer.perform()?;
        drop(transfer);

        let res = handle.response_code()?;

        Ok((res, buf))
    }

    fn login(email: &String, password: &String) -> Result<SkyPort, Error> {
        let body = format!("{{ \"email\": \"{}\", \"password\": \"{}\"}}", *email, *password);
        let url = "https://api.daikinskyport.com/users/auth/login";
        let (res, buf) = match access_webapi(url, None, Some(&body)) {
            Ok(t) => t,
            Err(e) => {
                return Err(Error::HTTPError(e));
            }
        };

        if res / 100 == 4 {
            let err: APIError = serde_json::from_slice(&buf[..]).unwrap();
            return Err(Error::APIError(res, err.message));
        }
        assert_eq!(res, 200);

        let result: LoginResult = serde_json::from_slice(&buf[..]).unwrap();
        if result.refreshToken.is_none() {
            return Err(Error::APIError(404 /* TODO */, "Refresh token was not returned".to_string()))
        }

        let skyport = SkyPort {
            email: email.clone(),
            access_token: result.accessToken,
            refresh_token: result.refreshToken.unwrap(),
            device_id: String::new(),
            device_data: DeviceData { ..Default::default() },
        };

        return Ok(skyport);
    }

    impl SkyPort {
        pub fn new(email: &String, password: &String) -> Result<SkyPort, Error> {
            let mut skyport = login(email, password)?;
            let (res, buf) = match access_webapi("https://api.daikinskyport.com/devices", Some(&skyport.access_token), None) {
                Ok(t) => t,
                Err(e) => {
                    return Err(Error::HTTPError(e));
                }
            };

            assert_eq!(res, 200);
            let devlist: Vec<DeviceEntry> = serde_json::from_slice(&buf[..]).unwrap();
            if devlist.len() == 0 {
                return Err(Error::APIError(404, "No device found".to_string()));
            }
            for dev in devlist.iter() {
                eprintln!("device id={}, name={}", dev.id, dev.name);
            }
            eprintln!("Using \"{}\" as a Daikin device", devlist[0].name);
            skyport.device_id = devlist[0].id.clone();

            skyport.do_sync()?;

            return Ok(skyport);
        }

        fn refresh_token(self: &mut SkyPort) -> Result<(), Error> {
            let url = "https://api.daikinskyport.com/users/auth/token";
            let body = format!("{{ \"email\": \"{}\", \"refreshToken\": \"{}\"}}", self.email, self.refresh_token);
            let (res, buf) = match access_webapi(url, None, Some(&body)) {
                Ok(t) => t,
                Err(e) => {
                    return Err(Error::HTTPError(e));
                }
            };
            assert_eq!(res, 200);

            let result: LoginResult = serde_json::from_slice(&buf[..]).unwrap();
            self.access_token = result.accessToken;

            return Ok(());
        }

        fn do_sync(self: &mut SkyPort) -> Result<(), Error> {
            let url = format!("https://api.daikinskyport.com/deviceData/{}", self.device_id);
            let (res, buf) = match access_webapi(&url, Some(&self.access_token), None) {
                Ok(t) => t,
                Err(e) => {
                    return Err(Error::HTTPError(e));
                }
            };
            assert_eq!(res, 200);

            let data: DeviceData = serde_json::from_slice(&buf[..]).unwrap();
            self.device_data = data;

            return Ok(());
        }

        pub fn sync(self: &mut SkyPort) -> Result<(), Error> {
            self.refresh_token()?;
            self.do_sync()
        }

        pub fn get_temp(self: &mut SkyPort) -> f64 {
            return self.device_data.temp_indoor;
        }
    }

    #[test]
    fn login_failure_test() {
        let mut res = SkyPort::new(&"crisp.fujita@gmail.com".to_owned(), &"hoge".to_owned());
        assert!(res.is_err());
    }

    #[test]
    fn device_parse_test () {
        let json = r#"
        [{"id":"23334be2-f495-4c1a-8b60-37ef44cd783b","locationId":"718b63d9-359f-471f-96d9-0923da5773e1","name":"Main Room","model":"ONEPLUS","firmwareVersion":"2.6.5","createdDate":1639528963,"hasOwner":true,"hasWrite":true}]
        "#;
        let devlist: Vec<DeviceEntry> = serde_json::from_str(&json).unwrap();
        assert!(devlist.len() == 1);
        assert_eq!(devlist[0].name, "Main Room");
    }
}

#[derive(Debug, Deserialize, Serialize)]
pub struct Config {
    #[serde(rename = "awair.deviceType")]
    awair_device_type: String,
    #[serde(rename = "awair.deviceId")]
    awair_device_id: u64,
    #[serde(rename = "awair.token")]
    awair_token: String,
    #[serde(rename = "targetTemp")]
    target_temp: f64,
    #[serde(rename = "daikin.email")]
    daikin_email: String,
    #[serde(rename = "daikin.password")]
    daikin_password: String,    
}

enum TimeRange {
    Contiguous(NaiveTime, NaiveTime),
    Split(NaiveTime, NaiveTime),
}

impl TimeRange {
    fn contains(self: &TimeRange, t: &NaiveTime) -> bool {
        match self {
            TimeRange::Contiguous(begin, end) => {
                begin <= t && t <= end
            },
            TimeRange::Split(end, begin) => {
                t <= end || begin <= t
            },
        }
    }
}

fn to_next(t: &NaiveTime, begin: &NaiveTime, end: &NaiveTime) -> i64 {
    if t < begin {
        (*begin - *t).num_seconds()
    } else if t < end {
        (*end - *t).num_seconds()
    } else {
        (Duration::hours(24) - (*t - *begin)).num_seconds()
    }
}

fn next_transition(t: &NaiveTime, range: &TimeRange) -> i64 {
    match range {
        TimeRange::Contiguous(begin, end) => {
            to_next(t, begin, end)
        },
        TimeRange::Split(end, begin) => {
            to_next(t, end, begin)
        }
    }
}

fn parse_time_range(begins: &str, ends: &str) -> TimeRange {
    let begint = NaiveTime::parse_from_str(&begins, "%R").unwrap();
    let endt = NaiveTime::parse_from_str(&ends, "%R").unwrap();
    if begint < endt {
        TimeRange::Contiguous(begint, endt)
    } else {
        TimeRange::Split(endt, begint)
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn time_range() {
        let range = parse_time_range("08:00", "13:00");
        assert!(matches!(range, TimeRange::Contiguous {..}));
        assert_eq!(range.contains(&NaiveTime::parse_from_str("12:00", "%R").unwrap()), true);
        assert_eq!(range.contains(&NaiveTime::parse_from_str("07:59", "%R").unwrap()), false);
        assert_eq!(next_transition(&NaiveTime::parse_from_str("07:00", "%R").unwrap(), &range), 60*60);
        assert_eq!(next_transition(&NaiveTime::parse_from_str("08:00", "%R").unwrap(), &range), 5*60*60);
        assert_eq!(next_transition(&NaiveTime::parse_from_str("11:00", "%R").unwrap(), &range), 2*60*60);
        assert_eq!(next_transition(&NaiveTime::parse_from_str("23:00", "%R").unwrap(), &range), 9*60*60);

        let range = parse_time_range("23:00", "07:00");
        assert!(matches!(range, TimeRange::Split {..}));
        assert_eq!(range.contains(&NaiveTime::parse_from_str("23:55", "%R").unwrap()), true);
        assert_eq!(range.contains(&NaiveTime::parse_from_str("00:00", "%R").unwrap()), true);
        assert_eq!(range.contains(&NaiveTime::parse_from_str("05:00", "%R").unwrap()), true);
        assert_eq!(next_transition(&NaiveTime::parse_from_str("23:30", "%R").unwrap(), &range), (7*60+30)*60);
        assert_eq!(next_transition(&NaiveTime::parse_from_str("04:00", "%R").unwrap(), &range), 3*60*60);

        let range = parse_time_range("23:00", "00:00");
        assert!(matches!(range, TimeRange::Split {..}));
        assert_eq!(range.contains(&NaiveTime::parse_from_str("23:55", "%R").unwrap()), true);
        assert_eq!(range.contains(&NaiveTime::parse_from_str("00:01", "%R").unwrap()), false);

        let range = parse_time_range("00:00", "11:00");
        assert!(matches!(range, TimeRange::Contiguous {..}));
        assert_eq!(range.contains(&NaiveTime::parse_from_str("06:00", "%R").unwrap()), true);
        assert_eq!(range.contains(&NaiveTime::parse_from_str("23:55", "%R").unwrap()), false);
    }

    #[test]
    fn curl() {
        let token = "eyJ0eXAiOiJKV1QiLCJhbGciOiJIUzI1NiJ9.eyJ1c2VyX2lkIjoiRFVNTVktSE9CQllJU1QifQ.hzjhIpGljqCZ8vCrOr89POy_ENDPYQXsnzGslP01krI";
        let temp = awair::read_average_temp("awair", 0, &token).unwrap();
        assert!((temp - 69.45).abs() < 0.01);
    }

    #[test]
    fn awair_parse() {
        let awair_json = r#"
        {
            "data": [
                {
                    "timestamp": "2022-01-02T06:30:00.000Z",
                    "score": 95.0,
                    "sensors": [
                        {
                            "comp": "pm25",
                            "value": 3.7
                        },
                        {
                            "comp": "humid",
                            "value": 41.932333119710286
                        },
                        {
                            "comp": "co2",
                            "value": 588.4
                        },
                        {
                            "comp": "temp",
                            "value": 24.175666745503744
                        },
                        {
                            "comp": "voc",
                            "value": 344.8666666666667
                        }
                    ],
                    "indices": [
                        {
                            "comp": "co2",
                            "value": 0.0
                        },
                        {
                            "comp": "temp",
                            "value": 0.0
                        },
                        {
                            "comp": "humid",
                            "value": 0.0
                        },
                        {
                            "comp": "pm25",
                            "value": 0.0
                        },
                        {
                            "comp": "voc",
                            "value": 0.9
                        }
                    ]
                },
                {
                    "timestamp": "2022-01-02T06:15:00.000Z",
                    "score": 94.97727272727273,
                    "sensors": [
                        {
                            "comp": "pm25",
                            "value": 4.2727272727272725
                        },
                        {
                            "comp": "humid",
                            "value": 42.014659057963975
                        },
                        {
                            "comp": "co2",
                            "value": 585.6363636363636
                        },
                        {
                            "comp": "temp",
                            "value": 24.310227264057506
                        },
                        {
                            "comp": "voc",
                            "value": 372.4318181818182
                        }
                    ],
                    "indices": [
                        {
                            "comp": "co2",
                            "value": 0.0
                        },
                        {
                            "comp": "temp",
                            "value": 0.0
                        },
                        {
                            "comp": "humid",
                            "value": 0.0
                        },
                        {
                            "comp": "pm25",
                            "value": 0.0
                        },
                        {
                            "comp": "voc",
                            "value": 1.0
                        }
                    ]
                },
                {
                    "timestamp": "2022-01-02T06:00:00.000Z",
                    "score": 94.0111111111111,
                    "sensors": [
                        {
                            "comp": "pm25",
                            "value": 4.433333333333334
                        },
                        {
                            "comp": "humid",
                            "value": 41.90066655476888
                        },
                        {
                            "comp": "co2",
                            "value": 588.6333333333333
                        },
                        {
                            "comp": "temp",
                            "value": 24.41155548095703
                        },
                        {
                            "comp": "voc",
                            "value": 489.23333333333335
                        }
                    ],
                    "indices": [
                        {
                            "comp": "co2",
                            "value": 0.0
                        },
                        {
                            "comp": "temp",
                            "value": 0.0
                        },
                        {
                            "comp": "humid",
                            "value": 0.0
                        },
                        {
                            "comp": "pm25",
                            "value": 0.0
                        },
                        {
                            "comp": "voc",
                            "value": 1.0
                        }
                    ]
                },
                {
                    "timestamp": "2022-01-02T05:45:00.000Z",
                    "score": 94.46666666666667,
                    "sensors": [
                        {
                            "comp": "pm25",
                            "value": 4.477777777777778
                        },
                        {
                            "comp": "humid",
                            "value": 41.97422235276964
                        },
                        {
                            "comp": "co2",
                            "value": 596.9777777777778
                        },
                        {
                            "comp": "temp",
                            "value": 24.31588887108697
                        },
                        {
                            "comp": "voc",
                            "value": 415.5444444444444
                        }
                    ],
                    "indices": [
                        {
                            "comp": "co2",
                            "value": 0.2777777777777778
                        },
                        {
                            "comp": "temp",
                            "value": 0.0
                        },
                        {
                            "comp": "humid",
                            "value": 0.0
                        },
                        {
                            "comp": "pm25",
                            "value": 0.0
                        },
                        {
                            "comp": "voc",
                            "value": 1.0
                        }
                    ]
                }
            ]
        }    
    "#;

        let data: awair::Data = serde_json::from_str(&awair_json).unwrap();
        assert!((awair::average_temp(&data) - 24.3).abs() < 0.01);
    }

    #[test]
    fn config_parse() {
        let config_json = r#"
        {
            "awair.deviceType": "awair",
            "awair.deviceId": 0,
            "awair.token": "eyJ0eXAiOiJKV1QiLCJhbGciOiJIUzI1NiJ9.eyJ1c2VyX2lkIjoiRFVNTVktSE9CQllJU1QifQ.hzjhIpGljqCZ8vCrOr89POy_ENDPYQXsnzGslP01krI",
            "targetTemp": 23.5,
            "daikin.email": "daikin@example.com",
            "daikin.password": "secret"
        }
        "#;
        let config: Config = serde_json::from_str(&config_json).unwrap();
        assert_eq!(config.awair_device_id, 0);
        assert_eq!(config.awair_device_type, "awair");
        assert!((config.target_temp - 23.5).abs() < 0.01);
    }

    #[test]
    fn daikin_test() {
        let config = read_config("config.json").unwrap();
        let mut daikin = daikin::SkyPort::new(&config.daikin_email, &config.daikin_password).unwrap();
        println!("temp={}", daikin.get_temp());
        daikin.sync().unwrap();
        println!("temp={}", daikin.get_temp());
    }
}

fn read_config(config_fn: &str) -> Result<Config, String> {
    let f = match std::fs::File::open(config_fn) {
        Ok(f) => f,
        Err(e) => {
            return Err(format!("Failed to open {}: {}", config_fn, e.to_string()));
        }
    };
    let buffered = std::io::BufReader::new(f);
    let config: Config = match serde_json::from_reader(buffered) {
        Ok(c) => c,
        Err(e) => {
            return Err(format!("Failed to parse {}: {}", config_fn, e.to_string()));
        }
    };
    Ok(config)
}

fn main() {
    let config = match read_config("config.json") {
        Ok(c) => c,
        Err(s) => {
            eprintln!("{}", s);
            std::process::exit(1);
        }
    };

    let range = parse_time_range("21:00", "07:00");
    let mut controlling = false;

    loop {
        /* TODO: handle error */
        let temp = awair::read_average_temp(&config.awair_device_type, config.awair_device_id, &config.awair_token).unwrap();
        println!("{}", temp);

        let now_dt = Local::now().naive_local();
        let now_t = now_dt.time();
        let next = next_transition(&now_t, &range) + 15;
        let in_range = range.contains(&now_t);
        if in_range != controlling {
            /* state transition */
            controlling = in_range;
        }

        if controlling {
            /* control Daikin */
            if (temp - config.target_temp).abs() > 0.5 {

            }
        }

        let sleep_sec = std::cmp::min(next, 10);
        println!("{} sleeping for {} seconds ({} minutes until next transition)", now_t, sleep_sec, next / 60);
        let dur = std::time::Duration::from_secs(sleep_sec.try_into().unwrap());
        std::thread::sleep(dur);
    }
}
