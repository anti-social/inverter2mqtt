use std::fs::File;
use std::io::BufReader;
use std::path::PathBuf;
use std::thread::sleep;
use std::time::Duration;

use clap::Parser;

use rusb::{devices, supports_detach_kernel_driver, UsbContext, DeviceHandle};

use paho_mqtt as mqtt;
use snafu::{Whatever, whatever, ResultExt};

mod config;
use crate::config::{Config, InverterConfig, MqttConfig, RequestParams, ResponseParams};
mod hass;
mod inverter;
use inverter::{DeviceError, Inverter, InverterDevice, MAX_COMMAND_LENGTH, SensorValue};

const INVERTER_QUERY_INTERVAL_SECS: u64 = 30;
const INVERTER_RETRY_DELAY_SECS: u64 = 10;
const MQTT_RETRY_DELAY_SECS: u64 = 10;
const MQTT_MIN_RETRY_INTERVAL_SECS: u64 = 1;
const MQTT_MAX_RETRY_INTERVAL_SECS: u64 = 60;

#[derive(Parser, Debug)]
struct Args {
    config: PathBuf,
}

struct InverterUSBDevice<T: UsbContext> {
    usb_dev: DeviceHandle<T>,
    request_params: RequestParams,
    response_params: ResponseParams,
}

impl<T: UsbContext> InverterUSBDevice<T> {
    fn new(
        usb_dev: DeviceHandle<T>,
        request_params: RequestParams,
        response_params: ResponseParams,
    ) -> Self {
        Self {
            usb_dev,
            request_params,
            response_params,
        }
    }
}

impl<T: UsbContext> InverterDevice for InverterUSBDevice<T> {
    fn send_request(&mut self, buf: &[u8]) -> Result<usize, DeviceError> {
        self.usb_dev.write_control(
            self.request_params.request_type,
            self.request_params.request,
            self.request_params.value,
            self.request_params.index,
            buf,
            Duration::from_millis(self.request_params.timeout_ms.into())
        ).map_err(|e| DeviceError::Usb { source: e })
    }

    fn read_response(&mut self, buf: &mut [u8]) -> Result<usize, DeviceError> {
        self.usb_dev.read_bulk(
            self.response_params.endpoint,
            buf,
            Duration::from_millis(self.response_params.timeout_ms.into())
        ).map_err(|e| DeviceError::Usb { source: e })
    }
}

fn main() -> Result<(), Whatever> {
    env_logger::init();

    let args = Args::parse();

    let config_file = File::open(args.config)
        .with_whatever_context(|e| format!("Cannot open config file: {e}"))?;
    let config_reader = BufReader::new(config_file);
    let config: Config = serde_yaml::from_reader(config_reader)
        .with_whatever_context(|e| format!("Error when parsing config file: {e}"))?;
    // Check commands length
    for command in config.inverter.commands.iter() {
        let cmd = &command.command;
        if cmd.len() > MAX_COMMAND_LENGTH {
            whatever!("'{cmd}' command is too long, maximum {MAX_COMMAND_LENGTH} chars");
        }
    }

    if !supports_detach_kernel_driver() {
        whatever!("Detaching kernel driver from USB device is not supported");
    }

    let dev_list = devices()
        .with_whatever_context(|e| format!("Error when fetching USB devices: {e}"))?;
    let mut dev_iter = dev_list.iter();
    let dev = loop {
        if let Some(dev) = dev_iter.next() {
            let dev_descr = dev.device_descriptor()
                .with_whatever_context(|e| format!("Error getting USB device descriptor: {e}"))?;
            let vendor_id = config.inverter.usb.vendor_id;
            let product_id = config.inverter.usb.product_id;
            if (dev_descr.vendor_id(), dev_descr.product_id()) == (vendor_id, product_id) {
                log::info!(
                    "Found device: {}:{}",
                    &format!("{:#06x}", vendor_id)[2..],
                    &format!("{:#06x}", product_id)[2..],
                );
                break Some((dev, dev_descr.max_packet_size()));
            }
        } else {
            break None;
        }
    };


    loop {
        // TODO: Take into account maximum packet size
        if let Some((dev, _max_packet_size)) = dev {
            let mut dev = dev.open()
                .with_whatever_context(|e| format!("Cannot open USB device: {e}"))?;
            dev.set_auto_detach_kernel_driver(true)
               .with_whatever_context(|e| format!("Cannot detach USB kernel driver: {e}"))?;
            dev.claim_interface(config.inverter.usb.interface)
               .with_whatever_context(|e| format!("Cannot claim USB interface: {e}"))?;

            let dev = InverterUSBDevice::new(
                dev,
                config.inverter.usb.request_params.clone(),
                config.inverter.usb.response_params.clone()
            );
            let mut inverter = Inverter::new(dev);
            let mqtt_client = establish_mqtt_conn(&config.mqtt)?;
            return run(&mut inverter, &config.inverter, &mqtt_client);
        } else {
            log::warn!("Devices are not found. Waiting");
            sleep(Duration::from_secs(INVERTER_RETRY_DELAY_SECS));
            continue;
        }
    }
}

fn establish_mqtt_conn(cfg: &MqttConfig) -> Result<mqtt::Client, Whatever> {
    let client = mqtt::Client::new(format!("tcp://{}", cfg.address))
        .with_whatever_context(|e| format!("Error creating mqtt client: {e}"))?;
    let mut conn_opts_builder = mqtt::ConnectOptionsBuilder::new();
    conn_opts_builder
        .keep_alive_interval(
            Duration::from_secs(INVERTER_QUERY_INTERVAL_SECS * 2)
        )
        .automatic_reconnect(
            Duration::from_secs(MQTT_MIN_RETRY_INTERVAL_SECS),
            Duration::from_secs(MQTT_MAX_RETRY_INTERVAL_SECS)
        )
        .clean_session(true);
    if let Some(auth) = &cfg.auth {
        conn_opts_builder
            .user_name(&auth.user)
            .password(&auth.password);
    }
    let conn_opts = conn_opts_builder.finalize();

    loop {
        if let Err(e) = client.connect(conn_opts.clone()) {
            log::warn!("Unable to connect to mqtt server. Waiting:\n\t{e}");
            sleep(Duration::from_secs(MQTT_RETRY_DELAY_SECS));
        } else {
            return Ok(client);
        }
    }
}

fn create_entities(
    inverter_cfg: &InverterConfig,
    mqtt_client: &mqtt::Client,
    inverter_base_topic: &str,
) -> Result<(), Whatever> {
    for command in inverter_cfg.commands.iter() {
        for sensor in command.sensors.iter().filter_map(|s| s.as_ref()) {
            let entity_name = format!("{}_{}", inverter_cfg.id, sensor.name);
            let discovery_name = sensor.human_name.clone()
                .unwrap_or_else(||
                    sensor.name.split('_').map(capitalize).collect::<Vec<_>>().join(" ")
                );
            let entity_base_topic = format!(
                "{inverter_base_topic}/{entity_name}",
            );
            let entity_config_topic = format!("{entity_base_topic}/config");
            let hass_discovery = hass::Discovery {
                name: discovery_name,
                object_id: entity_name.to_string(),
                unique_id: entity_name.to_string(),
                state_topic: format!("{entity_base_topic}/state"),
                device: hass::Device {
                    name: inverter_cfg.name.clone(),
                    identifiers: vec![inverter_cfg.id.clone()],
                    manufacturer: inverter_cfg.manufacturer.clone(),
                    model: inverter_cfg.model.clone(),
                },
                device_class: sensor.device_class.to_string(),
                unit_of_measurement: sensor.unit_of_measurement.to_string(),
                icon: sensor.icon.to_string(),
            };
            let entity_msg = serde_json::to_string(&hass_discovery)
                .with_whatever_context(|e| format!("Error when serializing discovery message: {e}"))?;
            let discovery_msg = mqtt::Message::new_retained(
                entity_config_topic.clone(),
                entity_msg.clone(),
                0
            );
            loop {
                log::trace!("Sending message to {entity_config_topic}: {entity_msg}");
                match mqtt_client.publish(discovery_msg.clone()) {
                    Ok(()) => break,
                    Err(e) => {
                        log::warn!("Error when creating entity: {e}");
                        sleep(Duration::from_secs(MQTT_RETRY_DELAY_SECS));
                        continue;
                    }
                }
            }
        }
    }

    Ok(())
}

fn run<T: InverterDevice>(
    inverter: &mut Inverter<T>,
    inverter_cfg: &InverterConfig,
    mqtt_client: &mqtt::Client,
) -> Result<(), Whatever> {
    let inverter_base_topic = format!(
        "homeassistant/sensor/{}", &inverter_cfg.id
    );

    create_entities(inverter_cfg, &mqtt_client, &inverter_base_topic)?;

    loop {
        for cmd_config in inverter_cfg.commands.iter() {
            let sensors_data = match inverter.execute_command(&cmd_config) {
                Ok(resp) => resp,
                Err(e) => {
                    log::warn!("Error when executing command '{}': {e}", cmd_config.command);
                    sleep(Duration::from_secs(INVERTER_RETRY_DELAY_SECS));
                    continue;
                }
            };
            for sensor in cmd_config.sensors.iter().filter_map(|s| s.as_ref()) {
                let sensor_value = match sensors_data.get(&sensor.name) {
                    Some(v) => v,
                    None => {
                        log::warn!("Missing value for sensor: {}", &sensor.name);
                        continue;
                    }
                };
                let entity_name = format!("{}_{}", &inverter_cfg.id, &sensor.name);
                let entity_value = match sensor_value {
                    SensorValue::Integer(v) => format!("{v}"),
                    SensorValue::Float(v) => format!("{v}"),
                    SensorValue::String(v) => v.clone(),
                };
                let entity_state_topic = format!("{inverter_base_topic}/{entity_name}/state");
                let entity_state_msg = mqtt::Message::new(
                    entity_state_topic.clone(),
                    entity_value.clone(),
                    0
                );

                log::trace!("Sending message to {entity_state_topic}: {entity_value}");
                if let Err(e) = mqtt_client.publish(entity_state_msg) {
                    log::warn!("Cannot publish entity state: {e}");
                    break;
                }
            }
        }

        sleep(Duration::from_secs(INVERTER_QUERY_INTERVAL_SECS));
    }
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        None => String::new(),
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
    }
}
