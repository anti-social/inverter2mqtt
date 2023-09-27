use std::collections::HashMap;
use std::num::{ParseFloatError, ParseIntError};
use std::str::{self, Utf8Error};

use crc::{Crc, CRC_16_XMODEM};

use rusb::Error as UsbError;

use snafu::Snafu;
use snafu::prelude::*;

use crate::config::CommandConfig;
use crate::config::ValueType;

// Encoded command contains: command + 2 bytes crc + \r
// Maximum 8 bytes
pub const MAX_COMMAND_LENGTH: usize = 5;
const START_RESPONSE_MARKER: u8 = b'(';
const END_RESPONSE_MARKER: u8 = b'\r';

#[derive(Debug, PartialEq, Snafu)]
pub enum DeviceError {
    #[snafu(display("USB device error: {source}"))]
    Usb { source: UsbError },
}

#[derive(Debug, PartialEq, Snafu)]
pub enum InverterError {
    #[snafu(display("Device error: {source}"))]
    Device { source: DeviceError },

    #[snafu(display("Command too long: {cmd}"))]
    CommandTooLong { cmd: String },

    #[snafu(display("Missing response marker"))]
    MissingResponseMarker,

    #[snafu(display("Expected UTF-8: {source}"))]
    ExpectedUtf8 { source: Utf8Error },

    #[snafu(display("Parse response error: {source}"))]
    ParseResponse { source: ParseResponseError },

    #[snafu(display("Invalid crc, expected {expected} but was {actual}: '{data}'"))]
    InvalidCrc { expected: String, actual: String, data: String },
}

#[derive(Debug, PartialEq, Snafu)]
pub enum ParseResponseError {
    #[snafu(display("Expected float value for '{sensor}' sensor: {source}"))]
    ExpectedFloat { sensor: String, source: ParseFloatError },

    #[snafu(display("Expected integer value for '{sensor}' sensor: {source}"))]
    ExpectedInteger { sensor: String, source: ParseIntError },
}

#[derive(Debug, PartialEq)]
pub enum SensorValue {
    Integer(i64),
    Float(f64),
    String(String),
}

pub trait InverterDevice {
    fn send_request(&mut self, buf: &[u8]) -> Result<usize, DeviceError>;
    fn read_response(&mut self, buf: &mut [u8]) -> Result<usize, DeviceError>;
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

    fn encode_command(&self, cmd: &str) -> Result<Vec<u8>, InverterError> {
        let mut res = vec!();
        res.extend(cmd.bytes());
        if res.len() > MAX_COMMAND_LENGTH {
            return Err(InverterError::CommandTooLong { cmd: cmd.to_string() });
        }
        let crc = self.calc_crc(&res);

        res.push((crc >> 8) as u8);
        res.push((crc & 0xff) as u8);
        res.push(b'\r');
        if res.len() < 8 {
            res.resize(8, b'\0');
        }
        Ok(res)
    }

    fn send_command(&mut self, cmd: &str) -> Result<usize, InverterError> {
        log::trace!("Sending command to inverter: {cmd}");
        let cmd = self.encode_command(cmd)?;
        self.dev.send_request(&cmd)
            .context(DeviceSnafu)
    }

    fn read_response(&mut self) -> Result<String, InverterError> {
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
            let data_for_crc = &resp[..resp.len()-2];
            let actual_crc = ((resp[resp.len()-2] as u16) << 8) | resp[resp.len()-1] as u16;
            return Err(InverterError::InvalidCrc {
                expected: format!("{:#06x}", self.calc_crc(data_for_crc)),
                actual: format!("{actual_crc:#06x}"),
                data: String::from_utf8_lossy(data_for_crc).into_owned(),
            });
        }

        let resp = str::from_utf8(&resp[1..resp.len()-2])
            .context(ExpectedUtf8Snafu)?;
        log::trace!("Read inverter response: {resp}");
        Ok(resp.to_string())
    }

    pub fn execute_command(
        &mut self,
        cfg: &CommandConfig
    ) -> Result<HashMap<String, SensorValue>, InverterError> {
        self.send_command(&cfg.command)?;
        let resp = self.read_response()?;
        let mut sensors_data = HashMap::new();
        for (sensor, value) in cfg.sensors.iter().zip(resp.split_ascii_whitespace()) {
            if let Some(sensor) = sensor {
                let value = match sensor.value_type {
                    ValueType::Integer => SensorValue::Integer(
                        value.parse::<i64>()
                            .context(ExpectedIntegerSnafu { sensor: sensor.name.clone() })
                            .context(ParseResponseSnafu)?
                    ),
                    ValueType::Float => SensorValue::Float(
                        value.parse::<f64>()
                            .context(ExpectedFloatSnafu { sensor: sensor.name.clone() })
                            .context(ParseResponseSnafu)?
                    ),
                    ValueType::String => SensorValue::String(
                        value.to_string()
                    ),
                };
                sensors_data.insert(sensor.name.clone(), value);
            }
        }
        Ok(sensors_data)
    }
}

fn slice_trim_end_matches<T, F: Fn(&T) -> bool>(arr: &[T], f: F) -> &[T] {
    let mut res = arr;
    while res.len() > 0 && f(&res[res.len()-1]) {
        res = &res[..(res.len()-1)];
    }
    res
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use crate::config::{CommandConfig, SensorConfig, ValueType};
    use super::{
        DeviceError,
        Inverter,
        InverterDevice,
        InverterError,
        ParseResponseError,
        SensorValue,
    };

    const ENCODED_STATUS_CMD: &'static [u8] = &[81, 80, 73, 71, 83, 183, 169, 13];

    struct TestInverterDevice<'req, 'resp> {
        expected_request: &'req [u8],
        response: &'resp [u8],
    }

    impl<'req, 'resp> TestInverterDevice<'req, 'resp> {
        fn new(expected_request: &'req [u8], response: &'resp [u8]) -> Self {
            Self {
                expected_request,
                response,
            }
        }
    }

    impl<'req, 'resp> InverterDevice for TestInverterDevice<'req, 'resp> {
        fn send_request(&mut self, buf: &[u8]) -> Result<usize, DeviceError> {
            assert_eq!(buf, self.expected_request);
            Ok(0)
        }

        fn read_response(&mut self, buf: &mut [u8]) -> Result<usize, DeviceError> {
            let (cur_resp, rest) = self.response.split_at(buf.len());
            buf.copy_from_slice(cur_resp);
            self.response = rest;
            Ok(buf.len())
        }
    }

    #[test]
    fn test_inverter_execute_command() {
        let mut inverter = Inverter::new(
            TestInverterDevice::new(
                ENCODED_STATUS_CMD,
                &[
                    b'(', b'0', b' ', b'2', b'3', b'3', b'.', b'7',
                    0x09, 0xc7, 13, 0, 0, 0, 0, 0,
                ]
            )
        );
        let command_config = CommandConfig {
            command: "QPIGS".to_string(),
            sensors: vec!(
                None,
                Some(
                    SensorConfig {
                        name: "sensor1".to_string(),
                        human_name: None,
                        value_type: ValueType::Float,
                        device_class: "voltage".to_string(),
                        unit_of_measurement: "V".to_string(),
                        icon: "mdi:power-plug".to_string(),
                    }
                )
            ),
        };
        let mut expected_result = HashMap::new();
        expected_result.insert("sensor1".to_string(), SensorValue::Float(233.7));
        assert_eq!(
            inverter.execute_command(&command_config).unwrap(),
            expected_result
        );
    }

    #[test]
    fn test_inverter_execute_command_invalid_crc() {
        let mut inverter = Inverter::new(
            TestInverterDevice::new(
                ENCODED_STATUS_CMD,
                &[
                    b'(', b'0', b' ', b'2', b'3', b'3', b'.', b'7',
                    0x09, 0xc8, 13, 0, 0, 0, 0, 0,
                ]
            )
        );
        let command_config = CommandConfig {
            command: "QPIGS".to_string(),
            sensors: vec!(None)
        };
        assert_eq!(
            inverter.execute_command(&command_config).unwrap_err(),
            InverterError::InvalidCrc {
                expected: "0x09c7".to_string(),
                actual: "0x09c8".to_string(),
                data: "(0 233.7".to_string(),
            }
        );
    }

    #[test]
    fn test_inverter_execute_command_invalid_value() {
        let mut inverter = Inverter::new(
            TestInverterDevice::new(
                ENCODED_STATUS_CMD,
                &[
                    b'(', b'a', 0xf3, 0xc8, 13, 0, 0, 0,
                ]
            )
        );
        let command_config = CommandConfig {
            command: "QPIGS".to_string(),
            sensors: vec!(
                Some(
                    SensorConfig {
                        name: "sensor1".to_string(),
                        human_name: None,
                        value_type: ValueType::Float,
                        device_class: "voltage".to_string(),
                        unit_of_measurement: "V".to_string(),
                        icon: "mdi:power-plug".to_string(),
                    }
                )
            ),
        };
        assert_eq!(
            inverter.execute_command(&command_config).unwrap_err(),
            InverterError::ParseResponse {
                source: ParseResponseError::ExpectedFloat {
                    sensor: "sensor1".to_string(),
                    source: "a".parse::<f64>().unwrap_err()
                }
            }
        );
    }
}
