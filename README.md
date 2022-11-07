# Daikawa - Controlling Daikin One+ Thermostat using Awair Element as a Remote Sensor

This program controls [Daikin One+](https://www.daikinone.com/) thermostat, using [Awair Element](https://www.getawair.com/products/element) as a remote temperature sensor. One of the (and probably the biggest) issues with Daikin One+ is that it does not come with or support a remote temperature sensor. This is a huge headache for a house where temperature disparity exists across rooms, for example multi-story homes. This program reads room temperature from Awair Element, then sets Daikin One+'s target temperature so that the temperature around Awair Element gets closer to desired one.

## Prerequisite

You will need a computer system that keeps running 24/7 to host Daikawa. It must be connected to the Internet, but it does not have to be in the same subnet with Awair Element or Daikin One+. All the controls (temperature reading/setting) are done through API server provided by Daikin and Awair.

Daikawa is a simple command-line tool written in Rust. It should work on most of platforms Rust supports, while I have only tested it on macOS/arm64 and Ubuntu 20.04/amd64. Rust compiler and standard toolchain around Rust (e.g. Cargo) are needed to build Daikawa.

You will need two credentials:
* E-mail address and password for Daikin One+. You should already have it if you set up Daikin One Home app in your phone.
* [Awair access token](https://developer.getawair.com/console/access-token)

Last but not least, your Awair Element must be in a place where airflow from Daikin's HVAC system reaches. Daikawa assumes there is a correlation between temperature around Daikin One+ and Awair Element.

## Install

Simply copy a binary to a desired path. Or you could use
```
cargo install --root=$PREFIX --path=.
```

## Configuration

Configuration is given by a TOML file. Example is given under the `example` directory.

## Run

The simplest way to invoke Daikawa is:
```
daikawa -c path/to/config.toml
```

Type
```
daikawa -h
```
for more options.

## systemd (optional)

It might be useful to run Daikawa as a systemd service (daemon), so it starts automatically when a system starts up. A sample configuration file for such a service is given under `example`.

```
sudo cp example/daikawa.service.example /etc/systemd/system/daikawa.service
(edit /etc/systemd/system/daikawa.service for your setup)
sudo chmod +x /etc/systemd/system/daikawa.service
sudo systemctl daemon-reload
sudo systemctl enable daikawa
sudo systemctl start daikawa
```

Once the service starts running, you can inspect Daikawa's log using
```
journalctl -u daikawa
```

## Acknowledgment

[daikinskyport](https://github.com/apetrycki/daikinskyport) helped me a lot in understanding Daikin's (undocumented) API usage. I really appreciate their effort.

## Contact

Issues and Pull Requests are always welcome. Because I'm a Rust newbie, Rust nitpicking and style suggestions are also greatly appreciated. You can also find me on [Twitter](https://twitter.com/fujita_d_h).
