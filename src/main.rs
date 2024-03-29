use serde::{Deserialize, Serialize};
use chrono::{Local, NaiveTime, Duration};
use getopts::Options;
use std::io::{Read};

#[derive(Debug, Deserialize, Serialize)]
struct APIError {
    message: String,
}

#[derive(Debug)]
pub enum Error {
    HTTPError(curl::Error),
    APIError(u32, String),
    GenericError(String),
}

/* error codes - must be >= 1000 to distinguish from HTTP status code */
const ERROR_STALE_DATA: u32 = 1000;

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::APIError(_, m) => {
                return write!(f, "{}", m);
            },
            Error::HTTPError(e) => {
                return write!(f, "{}", e.description());
            },
            Error::GenericError(s) => {
                return write!(f, "{}", s)
            },
        };
    }
}

mod webapi {
    use curl::easy::{Easy, List};

    pub enum HTTPMethod {
        GET,
        POST,
        PUT,
    }

    pub fn access(url: &str, method: HTTPMethod, token: Option<&String>, body: Option<&String>) -> Result<(u32, Vec<u8>), curl::Error> {
        let mut handle = Easy::new();
        let mut down_buf: Vec<u8> = Vec::new();
        handle.url(url)?;
        let mut list = List::new();
        list.append("Accept: application/json")?;
        list.append("Content-Type: application/json")?;
        if let Some(token) = token {
            let auth = format!("Authorization: Bearer {}", token);
            list.append(&auth)?;
        }
        handle.http_headers(list)?;

        match method {
            HTTPMethod::POST => {
                handle.post(true)?;
                assert!(body.is_some()); /* POST must have a body to upload */
                handle.post_fields_copy(body.unwrap().clone().into_bytes().as_slice())?;
            },
            HTTPMethod::PUT => {
                assert!(body.is_some()); /* PUT must have a body to upload */
                let up_buf = body.unwrap().as_bytes();
                handle.upload(true)?;
                handle.in_filesize(up_buf.len() as u64)?;
            },
            _ => ()
        }

        let mut transfer = handle.transfer();
        transfer.write_function(|data| {
            down_buf.extend_from_slice(data);
            Ok(data.len())
        })?;
        transfer.read_function(|into| {
            let up_buf = body.unwrap().as_bytes();
            let len = up_buf.len() as usize;
            into[0..len].clone_from_slice(up_buf);
            Ok(len)
        })?;
        transfer.perform()?;
        drop(transfer);

        let res = handle.response_code()?;

        Ok((res, down_buf))
    }
}

mod awair {
    use serde::{Deserialize, Serialize};
    use super::webapi;
    use super::Error;
    use super::APIError;
    use chrono::Local;
    use super::*;

    #[derive(Debug, Deserialize, Serialize)]
    struct Device {
        name: String,
        #[serde(rename = "deviceType")]
        device_type: String,
        #[serde(rename = "deviceId")]
        device_id: u64,
        #[serde(rename = "roomType")]
        room_type: String,
        #[serde(rename = "locationName")]
        location_name: String,
    }

    #[derive(Debug, Deserialize, Serialize)]
    struct Devices {
        devices: Vec<Device>,
    }

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

    fn get_latest_timestamp(data: &Data) -> chrono::DateTime<chrono::Local> {
        let uts = chrono::DateTime::parse_from_rfc3339(&data.data[0].timestamp).unwrap();
        return uts.with_timezone(&Local::now().timezone());
    }

    fn get_devices(token: &String) -> Result<Vec<Device>, Error> {
        let url = "https://developer-apis.awair.is/v1/users/self/devices";
        let (res, buf) = match webapi::access(&url, webapi::HTTPMethod::GET, Some(token), None) {
            Ok(r) => r,
            Err(e) => {
                return Err(Error::HTTPError(e));
            }
        };

        if res != 200 {
            let r: serde_json::Result<APIError> = serde_json::from_slice(&buf);
            match r {
                Ok(ae) => {
                    return Err(Error::APIError(res, ae.message));
                },
                _ => {
                    return Err(Error::APIError(res, String::from_utf8(buf).unwrap_or_default()));
                }
            }
        }

        let result: Devices = serde_json::from_slice(&buf).unwrap();

        if result.devices.len() == 0 {
            return Err(Error::APIError(1404, "No device defined".to_string()));
        }

        Ok(result.devices)
    }

    #[test]
    fn test_get_devices() {
        let token = "eyJ0eXAiOiJKV1QiLCJhbGciOiJIUzI1NiJ9.eyJ1c2VyX2lkIjoiRFVNTVktSE9CQllJU1QifQ.hzjhIpGljqCZ8vCrOr89POy_ENDPYQXsnzGslP01krI";
        let _ = get_devices(&token.to_string());
    }

    pub struct Awair {
        token: String,
        device_type: String,
        device_id: u64,
    }

    impl Awair {
        pub fn new(token: &String) -> Result<Awair, Error> {
            let devices = get_devices(token)?;
            println!("Selecting Awair device: name=\"{}\", deviceType=\"{}\", deviceId={}, roomType=\"{}\", locationName=\"{}\"",
                devices[0].name, devices[0].device_type, devices[0].device_id, devices[0].room_type, devices[0].location_name);
            let awair = Awair {
                token: token.clone(),
                device_type: devices[0].device_type.clone(),
                device_id: devices[0].device_id,
            };
            Ok(awair)
        }

        pub fn get_temp(&self) -> Result<f64, Error> {
            let url = format!("https://developer-apis.awair.is/v1/users/self/devices/{}/{}/air-data/latest", self.device_type, self.device_id);
            let (res, buf) = match webapi::access(&url, webapi::HTTPMethod::GET, Some(&self.token), None) {
                Ok(r) => r,
                Err(e) => {
                    return Err(Error::HTTPError(e));
                }
            };

            if res != 200 {
                return Err(Error::APIError(res, String::from_utf8(buf).unwrap_or_default()));
            }

            let data: Data = match serde_json::from_slice(&buf[..]) {
                Ok(d) => d,
                /* `latest` could return empty json if the latest data is not available */
                Err(e) => return Err(Error::GenericError(e.to_string())),
            };
            if (Local::now() - get_latest_timestamp(&data)).num_minutes() > 15 {
                return Err(Error::APIError(ERROR_STALE_DATA, "Stale data".to_string()));
            }
            /* in case of `latest` we actually get average, but we call `average_temp` here just to traverse returned json */
            return Ok(average_temp(&data));
        }
    }

    #[cfg(test)]
    #[test]
    fn test_new() {
        let token = "eyJ0eXAiOiJKV1QiLCJhbGciOiJIUzI1NiJ9.eyJ1c2VyX2lkIjoiRFVNTVktSE9CQllJU1QifQ.hzjhIpGljqCZ8vCrOr89POy_ENDPYQXsnzGslP01krI";
        let _ = Awair::new(&token.to_string()).unwrap();
    }
}

mod daikin {
    use serde::{Deserialize, Serialize};
    use super::webapi;
    use super::Error;
    use super::APIError;

    pub struct SkyPort {
        email: String,
        access_token: String,
        refresh_token: String,
        device_id: String,
        device_data: DeviceData,
    }

    #[derive(Debug, Deserialize, Serialize)]
    struct LoginResult {
        #[serde(rename = "accessToken")]
        access_token: String,
        #[serde(rename = "accessTokenExpiresIn")]
        access_token_expires_in: u64,
        #[serde(rename = "refreshToken")]
        refresh_token: Option<String>,
        #[serde(rename = "tokenType")]
        token_type: String,
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
        #[serde(rename = "geofencingAway")]
        geofencing_away: bool,
        #[serde(rename = "tempOutdoor")]
        temp_outdoor: f64,
    }

    fn login(email: &String, password: &String) -> Result<SkyPort, Error> {
        let body = format!("{{ \"email\": \"{}\", \"password\": \"{}\"}}", *email, *password);
        let url = "https://api.daikinskyport.com/users/auth/login";
        let (res, buf) = match webapi::access(url, webapi::HTTPMethod::POST, None, Some(&body)) {
            Ok(t) => t,
            Err(e) => {
                return Err(Error::HTTPError(e));
            }
        };

        if res != 200 {
            let message = if let Ok(err) = serde_json::from_slice::<APIError>(&buf[..]) {
                err.message
            } else {
                "Unknown error".to_string()
            };
            return Err(Error::APIError(res, message));
        }

        let result: LoginResult = match serde_json::from_slice(&buf[..]) {
            Ok(r) => r,
            _ => return Err(Error::GenericError(format!("Could not parse login result: {:?}", buf)))
        };
        if result.refresh_token.is_none() {
            return Err(Error::GenericError("Refresh token was not returned".to_string()))
        }

        let skyport = SkyPort {
            email: email.clone(),
            access_token: result.access_token,
            refresh_token: result.refresh_token.unwrap(), /* this is safe because we already checked is_none above */
            device_id: String::new(),
            device_data: DeviceData { ..Default::default() },
        };

        return Ok(skyport);
    }

    impl SkyPort {
        pub fn new(email: &String, password: &String) -> Result<SkyPort, Error> {
            let mut skyport = login(email, password)?;
            let (res, buf) = match webapi::access("https://api.daikinskyport.com/devices", webapi::HTTPMethod::GET, Some(&skyport.access_token), None) {
                Ok(t) => t,
                Err(e) => {
                    return Err(Error::HTTPError(e));
                }
            };

            if res != 200 {
                return Err(Error::APIError(res, format!("Login failed: {}", res)));
            }
            let devlist: Vec<DeviceEntry> = match serde_json::from_slice(&buf[..]) {
                Ok(l) => l,
                Err(e) => return Err(Error::GenericError(e.to_string())),
            };
            if devlist.len() == 0 {
                return Err(Error::APIError(404, "No device found".to_string()));
            }
            for dev in devlist.iter() {
                eprintln!("Daikin Skyport: found device id={}, name={}", dev.id, dev.name);
            }
            eprintln!("Daikin Skyport: Using \"{}\" as a Daikin device", devlist[0].name);
            skyport.device_id = devlist[0].id.clone();

            skyport.do_sync()?;

            return Ok(skyport);
        }

        fn refresh_token(self: &mut SkyPort) -> Result<(), Error> {
            let url = "https://api.daikinskyport.com/users/auth/token";
            let body = format!("{{ \"email\": \"{}\", \"refreshToken\": \"{}\"}}", self.email, self.refresh_token);
            let (res, buf) = match webapi::access(url, webapi::HTTPMethod::POST, None, Some(&body)) {
                Ok(t) => t,
                Err(e) => {
                    return Err(Error::HTTPError(e));
                }
            };

            if res != 200 {
                return Err(Error::APIError(res, String::from_utf8(buf).unwrap_or_default()));
            }

            let result: LoginResult = serde_json::from_slice(&buf[..]).unwrap();
            self.access_token = result.access_token;

            return Ok(());
        }

        fn do_sync(self: &mut SkyPort) -> Result<(), Error> {
            let url = format!("https://api.daikinskyport.com/deviceData/{}", self.device_id);
            let (res, buf) = match webapi::access(&url, webapi::HTTPMethod::GET, Some(&self.access_token), None) {
                Ok(t) => t,
                Err(e) => {
                    return Err(Error::HTTPError(e));
                }
            };

            if res != 200 {
                return Err(Error::APIError(res, String::from_utf8(buf).unwrap_or_default()));
            }

            let data: DeviceData = match serde_json::from_slice(&buf[..]) {
                Ok(d) => d,
                Err(e) => return Err(Error::GenericError(e.to_string())),
            };
            self.device_data = data;

            return Ok(());
        }

        pub fn sync(self: &mut SkyPort) -> Result<(), Error> {
            if let Err(e) = self.do_sync() {
                if let Error::APIError(401, _) = e {
                    self.refresh_token()?;
                    return self.do_sync();
                } else {
                    return Err(e);
                }
            }
            Ok(())
        }

        pub fn get_temp_indoor(self: &SkyPort) -> f64 {
            return self.device_data.temp_indoor;
        }

        pub fn get_heat_setpoint(self: &SkyPort) -> f64 {
            return self.device_data.hsp_home;
        }

        pub fn get_cool_setpoint(self: &SkyPort) -> f64 {
            return self.device_data.csp_home;
        }

        pub fn get_geofencing_away(self: &SkyPort) -> bool {
            return self.device_data.geofencing_away;
        }

        pub fn get_temp_outdoor(self: &SkyPort) -> f64 {
            return self.device_data.temp_outdoor;
        }

        fn do_set_setpoints(&self, heat: f64, cool: f64, duration: u32) -> Result<(), Error> {
            let url = format!("https://api.daikinskyport.com/deviceData/{}", self.device_id);
            let body = format!("{{\"hspHome\": {:.1}, \"cspHome\": {:.1}, \"schedOverride\": 1, \"schedOverrideDuration\": {}}}",
                heat, cool, duration);
            let (res, buf) = match webapi::access(&url, webapi::HTTPMethod::PUT, Some(&self.access_token), Some(&body)) {
                Ok(t) => t,
                Err(e) => {
                    return Err(Error::HTTPError(e));
                }
            };
            if res != 200 {
                return Err(Error::APIError(res, String::from_utf8(buf).unwrap_or_default()));
            }
            return Ok(());
        }

        pub fn set_setpoints(&mut self, heat: f64, cool: f64, duration: u32) -> Result<(), Error> {
            if let Err(e) = self.do_set_setpoints(heat, cool, duration) {
                if let Error::APIError(401, _) = e {
                    self.refresh_token()?;
                    return self.do_set_setpoints(heat, cool, duration);
                } else {
                    return Err(e);
                }
            }
            Ok(())
        }
    }

    #[ignore]
    #[test]
    fn login_failure_test() {
        let res = SkyPort::new(&"test@example.com".to_owned(), &"hoge".to_owned());
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
    #[serde(rename = "awair_token")]
    awair_token: String,
    target_temp_heat: f64,
    target_temp_cool: f64,
    control_start: String,
    control_end: String,
    #[serde(rename = "daikin_email")]
    daikin_email: String,
    #[serde(rename = "daikin_password")]
    daikin_password: String,
    #[serde(skip)]
    dry_run: bool,
    #[serde(skip)]
    oneshot: bool,
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
    fn timestamp_parse() {
        let ts = "2022-03-18T22:30:00.000Z";
        let dt = chrono::DateTime::parse_from_rfc3339(ts).unwrap();
        let ldt = dt.with_timezone(&Local::now().timezone());
        let lnow = Local::now();
        let diff = lnow - ldt;
        println!("{}", diff.num_minutes());
    }

    #[test]
    fn config_parse() {
        let config_json = r#"
        {
            "awair.deviceType": "awair",
            "awair.deviceId": 0,
            "awair.token": "eyJ0eXAiOiJKV1QiLCJhbGciOiJIUzI1NiJ9.eyJ1c2VyX2lkIjoiRFVNTVktSE9CQllJU1QifQ.hzjhIpGljqCZ8vCrOr89POy_ENDPYQXsnzGslP01krI",
            "target_temp_heat": 23.5,
            "target_temp_cool": 26.0,
            "control_start": "21:00",
            "control_end": "07:00",
            "daikin.email": "daikin@example.com",
            "daikin.password": "secret"
        }
        "#;
        let config: Config = serde_json::from_str(&config_json).unwrap();
        assert!((config.target_temp_heat - 23.5).abs() < 0.01);
    }

    #[ignore]
    #[test]
    fn daikin_test() {
        let config = read_config("config.toml").unwrap();
        let mut daikin = daikin::SkyPort::new(&config.daikin_email, &config.daikin_password).unwrap();
        println!("temp={}", daikin.get_temp_indoor());
        daikin.sync().unwrap();
        println!("temp={}", daikin.get_temp_indoor());
        daikin.set_setpoints(21.0, 26.0, 1).unwrap();
    }

    #[test]
    fn setpoint_calc() {
        let (h, c) = calc_new_setpoints(23.5, 21.0, 23.5, 26.0);
        assert!((c - 23.5).abs() < 0.01);
        assert!((h - 21.0).abs() < 0.01);

        let (h, c) = calc_new_setpoints(24.5, 21.5, 23.5, 26.0);
        assert!((c - 23.0).abs() < 0.01);
        assert!((h - 20.5).abs() < 0.01);

        let (h, c) = calc_new_setpoints(27.0, 23.0, 23.5, 26.0);
        assert!((c - 22.0).abs() < 0.01);
        assert!((h - 19.5).abs() < 0.01);
    }
}

fn read_config(config_fn: &str) -> Result<Config, String> {
    let f = match std::fs::File::open(config_fn) {
        Ok(f) => f,
        Err(e) => {
            return Err(format!("Failed to open {}: {}", config_fn, e.to_string()));
        }
    };
    let mut config_str = String::new();
    let mut buffered = std::io::BufReader::new(f);
    if let Err(e) = buffered.read_to_string(&mut config_str) {
        return Err(format!("Failed to read {}: {}", config_fn, e.to_string()));
    }
    let config: Config = match toml::from_str(&config_str) {
        Ok(c) => c,
        Err(e) => {
            return Err(format!("Failed to parse {}: {}", config_fn, e.to_string()));
        }
    };
    if config.target_temp_heat > config.target_temp_cool {
        return Err("target_temp_heat must be lower than or equal to target_temp_cool".to_owned());
    }
    Ok(config)
}

/**
 * returns (new_heat_setpoint, new_cool_setpoint)
 *
 * atemp: Awair temperature
 * dtemp: Daikin temperature
 */
fn calc_new_setpoints(atemp: f64, dtemp: f64, target_heat: f64, target_cool: f64) -> (f64, f64) {
    let diff = atemp - dtemp;
    let new_hsp = target_heat - diff;
    let new_csp = target_cool - diff;
    (new_hsp, new_csp)
}

#[derive(Serialize)]
struct TempLog {
    target_temp_heat: f64,
    target_temp_cool: f64,
    awair_temp: f64,
    daikin_indoor_temp: f64,
    daikin_outdoor_temp: f64,
    current_heat_setpoint: f64,
    current_cool_setpoint: f64,
    new_heat_setpoint: f64,
    new_cool_setpoint: f64,
    /** indicates if the new temperature settings are actually set to Daikin */
    execute_control: bool,
}

fn print_log(log: &TempLog) {
    if let Ok(str) = serde_json::to_string(log) {
        println!("{}", str);
    }
}

/**
 * Implements the main control logic
 * returns sleep interval until next execution (in minutes)
 */
fn do_control(awair: &awair::Awair, skyport: &mut daikin::SkyPort, config: &Config) -> u32 {
    let default = 15;
    let retry = 5;
    /* control Daikin */
    if let Err(e) = skyport.sync() {
        eprintln!("Daikin Skyport sync failed: {}", e);
        return retry;
    }

    let atemp = match awair.get_temp() {
        Ok(t) => t,
        Err(e) => {
            eprintln!("Failed to obtain Awair readings: {}, skipping control", e);
            return retry;
        }
    };
    let dtemp = skyport.get_temp_indoor();
    let (new_hsp, new_csp) = calc_new_setpoints(atemp, dtemp, config.target_temp_heat, config.target_temp_cool);

    let away = skyport.get_geofencing_away();
    let execute = !(away || config.dry_run);
    let log = TempLog {
        target_temp_heat: config.target_temp_heat,
        target_temp_cool:  config.target_temp_cool,
        awair_temp: atemp,
        daikin_indoor_temp: dtemp,
        daikin_outdoor_temp: skyport.get_temp_outdoor(),
        current_heat_setpoint: skyport.get_heat_setpoint(),
        current_cool_setpoint: skyport.get_cool_setpoint(),
        new_heat_setpoint: new_hsp,
        new_cool_setpoint: new_csp,
        execute_control: execute,
    };
    print_log(&log);

    if !execute {
        return default;
    }

    if let Err(e) = skyport.set_setpoints(new_hsp, new_csp, default) {
        eprintln!("Failed to set setpoints: {}", e);
        return retry;
    }

    return default;
}

fn print_usage(program: &str, opts: Options) {
    let brief = format!("Usage: {} [options]", program);
    print!("{}", opts.usage(&brief));
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let prog = &args[0];
    let mut opts = Options::new();
    opts.optopt("c", "config", "specify a configuration file (default: config.toml)", "FILE");
    opts.optflag("", "config-test", "read a configuration file and exit");
    opts.optflag("", "dry-run", "read sensor values but do not change temperature settings");
    opts.optflag("", "oneshot", "execute the control loop once and exit");
    opts.optflag("h", "help", "show this menu");
    let matches = match opts.parse(&args[1..]) {
        Ok(m) => m,
        Err(f) => {
            eprintln!("{}\n", f.to_string());
            print_usage(prog, opts);
            return;
        }
    };
    if matches.opt_present("h") {
        print_usage(prog, opts);
        return;
    }
    let config_file = match matches.opt_str("c") {
        Some(f) => f,
        None => "config.toml".to_string(),
    };

    let mut config = match read_config(&config_file) {
        Ok(c) => c,
        Err(s) => {
            eprintln!("{}", s);
            std::process::exit(1);
        }
    };

    if matches.opt_present("config-test") {
        return;
    }
    if matches.opt_present("dry-run") {
        config.dry_run = true;
    }
    if matches.opt_present("oneshot") {
        config.oneshot = true;
    }

    let range = parse_time_range(&config.control_start, &config.control_end);
    let mut controlling = false;

    let awair = match awair::Awair::new(&config.awair_token) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("Failed to create Awair object: {}", e);
            std::process::exit(1);
        }
    };

    let mut skyport = match daikin::SkyPort::new(&config.daikin_email, &config.daikin_password) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Failed to connect to Daikin Skyport: {}", e);
            std::process::exit(1);
        }
    };

    loop {
        let now_dt = Local::now().naive_local();
        let now_t = now_dt.time();
        let next = next_transition(&now_t, &range) + 15;
        let in_range = range.contains(&now_t);
        if in_range != controlling {
            /* state transition */
            controlling = in_range;
        }

        let interval_min = if controlling {
            do_control(&awair, &mut skyport, &config)
        } else {
            24*60 /* sleep forever */
        };

        if config.oneshot {
            return;
        }

        let sleep_sec = std::cmp::min(next, interval_min as i64 * 60);
        println!("sleeping for {} seconds ({} minutes until next state transition)", sleep_sec, next / 60);
        let dur = std::time::Duration::from_secs(sleep_sec.try_into().unwrap());
        std::thread::sleep(dur);
    }
}
