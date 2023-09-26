use std::str::{self, Utf8Error};

use crc::{Crc, CRC_16_XMODEM};

use rusb::Error as UsbError;

use serde::{Deserialize, Serialize};

use snafu::Snafu;
use snafu::prelude::*;

use crate::parse::{DeError as ParseError, from_str};

const START_RESPONSE_MARKER: u8 = b'(';
const END_RESPONSE_MARKER: u8 = b'\r';

#[derive(Debug, Snafu)]
pub enum DeviceError {
    #[snafu(display("USB device error: {source}"))]
    Usb { source: UsbError },
}

#[derive(Debug, Snafu)]
pub enum InverterError {
    #[snafu(display("Device error: {source}"))]
    Device { source: DeviceError },

    #[snafu(display("Missing response marker"))]
    MissingResponseMarker,

    #[snafu(display("Expected UTF-8: {source}"))]
    ExpectedUtf8 { source: Utf8Error },

    #[snafu(display("Parse response error: {source}"))]
    ParseResponse { source: ParseError },

    #[snafu(display("Invalid crc"))]
    InvalidCrc,
}

#[derive(Debug, Deserialize, Serialize, PartialEq)]
pub struct Status1 {
    grid_voltage: f32,
    grid_frequency: f32,
    out_voltage: f32,
    out_frequency: f32,
    load_va: f32,
    load_watt: f32,
    load_percent: f32,
    bus_voltage: f32,
    battery_voltage: f32,
    battery_charge_current: f32,
    battery_capacity: f32,
    heatsink_temperature: f32,
    pv_input_voltage: f32,
    scc_voltage: f32,
    battery_discharge_current: f32,
    device_status: String
}

impl Status1 {
    pub fn entity_state(&self, name: &str) -> Option<f32> {
        Some(
            match name {
                "grid_voltage" => self.grid_voltage,
                "grid_frequency" => self.grid_frequency,
                "out_voltage" => self.out_voltage,
                "out_frequency" => self.out_frequency,
                "load_va" => self.load_va,
                "load_watt" => self.load_watt,
                "load_percent" => self.load_percent,
                "bus_voltage" => self.bus_voltage,
                "battery_voltage" => self.battery_voltage,
                "battery_charge_current" => self.battery_charge_current,
                "battery_capacity" => self.battery_capacity,
                "heatsink_temperature" => self.heatsink_temperature,
                "pv_input_voltage" => self.pv_input_voltage,
                "scc_voltage" => self.scc_voltage,
                "battery_discharge_current" => self.battery_discharge_current,
                _ => return None,
            }
        )
    }
}

pub trait InverterDevice {
    fn send_request(&self, buf: &[u8]) -> Result<usize, DeviceError>;
    fn read_response(&self, buf: &mut [u8]) -> Result<usize, DeviceError>;
}

pub struct Inverter<T: InverterDevice> {
    dev: T,
}

impl<T: InverterDevice> Inverter<T> {
    pub fn new(dev: T) -> Self {
        Self {
            dev,
        }
    }

    fn calc_crc(&self, data: &[u8]) -> u16 {
        let crc = Crc::<u16>::new(&CRC_16_XMODEM);
        let mut digest = crc.digest();
        digest.update(data);
        digest.finalize()
    }

    fn encode_command(&self, cmd: &str) -> Vec<u8> {
        let cmd = cmd.as_bytes();
        let crc = self.calc_crc(cmd);

        let mut res = vec!();
        res.extend(cmd);
        res.push((crc >> 8) as u8);
        res.push((crc & 0xff) as u8);
        res.push(b'\r');
        if res.len() < 8 {
            res.resize(8, b'\0');
        }
        res
    }

    fn send_command(&self, cmd: &str) -> Result<usize, InverterError> {
        let cmd = self.encode_command(cmd);
        self.dev.send_request(&cmd)
            .context(DeviceSnafu)
    }

    fn read_response(&self) -> Result<String, InverterError> {
        let mut resp = Vec::<u8>::new();
        loop {
            let mut buf = [0; 8];
            self.dev.read_response(&mut buf)
                .context(DeviceSnafu)?;
            let chunk = slice_trim_end_matches(&buf, |&b| b == b'\0');
            resp.extend(chunk);
            if let Some(&END_RESPONSE_MARKER) = chunk.last() {
                resp.pop();
                break;
            }
        }

        if resp[0] != START_RESPONSE_MARKER {
            return Err(InverterError::MissingResponseMarker);
        }

        if self.calc_crc(&resp) != 0 {
            return Err(InverterError::InvalidCrc);
        }

        Ok(
            str::from_utf8(&resp[1..resp.len()-2])
                .context(ExpectedUtf8Snafu)?
                .to_string()
        )
    }

    pub fn status1(&self) -> Result<Status1, InverterError> {
        self.send_command("QPIGS")?;
        let resp = self.read_response()?;
        from_str(&resp).context(ParseResponseSnafu)
    }
}

fn slice_trim_end_matches<T, F: Fn(&T) -> bool>(arr: &[T], f: F) -> &[T] {
    let mut res = arr;
    while res.len() > 0 && f(&res[res.len()-1]) {
        res = &res[..(res.len()-1)];
    }
    res
}
