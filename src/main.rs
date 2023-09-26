use std::thread::sleep;
use std::time::Duration;

use rusb::{devices, supports_detach_kernel_driver, UsbContext, DeviceHandle};

mod inverter;
use inverter::{DeviceError, Inverter, InverterDevice};

use paho_mqtt as mqtt;
use snafu::{Whatever, whatever, ResultExt};

mod hass;
mod parse;

const POWMR_VENDOR: u16 = 0x0665;
const POWMR_PRODUCT: u16 = 0x5161;
const POWMR_INTERFACE: u8 = 0;
// TODO: Custom packet size
// const POWMR_PACKET_SIZE: usize = 8;
const POWMR_REQUEST_CONFIG: USBRequestConfig = USBRequestConfig {
    request_type: 0x21,
    request: 0x9,
    value: 0x200,
    index: 0,
    timeout: Duration::from_millis(100),

};
const POWMR_RESPONSE_CONFIG: USBResponseConfig = USBResponseConfig {
    endpoint: 0x81,
    timeout: Duration::from_millis(100),
};

struct USBRequestConfig {
    request_type: u8,
    request: u8,
    value: u16,
    index: u16,
    timeout: Duration,
}

struct USBResponseConfig {
    endpoint: u8,
    timeout: Duration,
}

struct PowmrUSBDevice<T: UsbContext> {
    usb_dev: DeviceHandle<T>,
    request_config: USBRequestConfig,
    response_config: USBResponseConfig,
}

impl<T: UsbContext> PowmrUSBDevice<T> {
    fn new(
        usb_dev: DeviceHandle<T>,
        request_config: USBRequestConfig,
        response_config: USBResponseConfig,
    ) -> Self {
        Self {
            usb_dev,
            request_config,
            response_config,
        }
    }
}

impl<T: UsbContext> InverterDevice for PowmrUSBDevice<T> {
    fn send_request(&self, buf: &[u8]) -> Result<usize, DeviceError> {
        self.usb_dev.write_control(
            self.request_config.request_type,
            self.request_config.request,
            self.request_config.value,
            self.request_config.index,
            buf,
            self.request_config.timeout
        ).map_err(|e| DeviceError::Usb { source: e })
    }

    fn read_response(&self, buf: &mut [u8]) -> Result<usize, DeviceError> {
        self.usb_dev.read_bulk(
            self.response_config.endpoint,
            buf,
            self.response_config.timeout
        ).map_err(|e| DeviceError::Usb { source: e })
    }
}

fn main() -> Result<(), Whatever> {
    if !supports_detach_kernel_driver() {
        whatever!("Detaching kernel driver from USB device is not supported");
    }

    let dev_list = devices().expect("USB devices");
    let mut dev_iter = dev_list.iter();
    let dev = loop {
        if let Some(dev) = dev_iter.next() {
            let dev_descr = dev.device_descriptor().expect("USB device descriptor");
            if (dev_descr.vendor_id(), dev_descr.product_id()) == (POWMR_VENDOR, POWMR_PRODUCT) {
                println!("Found the device with max packet size: {}", dev_descr.max_packet_size());
                break Some(dev);
            }
        } else {
            break None;
        }
    };


    if let Some(dev) = dev {
        let mut dev = dev.open()
            .with_whatever_context(|e| format!("Cannot open USB device: {e}"))?;
        dev.set_auto_detach_kernel_driver(true)
            .with_whatever_context(|e| format!("Cannot detach USB kernel driver: {e}"))?;
        dev.claim_interface(POWMR_INTERFACE)
            .with_whatever_context(|e| format!("Cannot claim USB interface: {e}"))?;

        let dev = PowmrUSBDevice::new(
            dev,
            POWMR_REQUEST_CONFIG,
            POWMR_RESPONSE_CONFIG
        );
        let inverter = Inverter::new(dev);
        run(inverter)
    } else {
        println!("Devices are not found. Waiting");
        Ok(())
    }
}

fn establish_mqtt_conn() -> Result<mqtt::Client, Whatever> {
    let client = mqtt::Client::new("tcp://localhost:1883")
        .with_whatever_context(|e| format!("Error creating mqtt client: {e}"))?;
    let conn_opts = mqtt::ConnectOptionsBuilder::new()
        .keep_alive_interval(Duration::from_secs(60))
        .clean_session(true)
        .finalize();

    loop {
        if let Err(e) = client.connect(conn_opts.clone()) {
            println!("Unable to connect to mqtt server. Waiting:\n\t{e}");
            sleep(Duration::from_secs(10));
        } else {
            return Ok(client);
        }
    }
}

fn create_entities(
    mqtt_client: &mqtt::Client,
    device: &hass::Device,
    inverter_base_topic: &str,
) -> Result<(), Whatever> {
    for sensor in hass::SENSORS {
        let entity_name = sensor.entity_name;
        let entity_base_topic = format!(
            "{inverter_base_topic}/{entity_name}",
        );
        let entity_config_topic = format!("{entity_base_topic}/config");
        let hass_discovery = hass::Discovery {
            name: entity_name.split('_').map(capitalize).collect::<Vec<_>>().join(" "),
            object_id: entity_name.to_string(),
            unique_id: entity_name.to_string(),
            state_topic: format!("{entity_base_topic}/state"),
            device: device.clone(),
            device_class: sensor.device_class.to_string(),
            unit_of_measurement: sensor.unit_of_measurement.to_string(),
            icon: sensor.icon.to_string(),
        };
        let discovery_msg = mqtt::Message::new(
            entity_config_topic,
            serde_json::to_string(&hass_discovery)
                .with_whatever_context(|e| format!("Error when serializing discovery message: {e}"))?,
            0
        );
        loop {
            match mqtt_client.publish(discovery_msg.clone()) {
                Ok(()) => break,
                Err(e) => {
                    println!("Error when creating entity: {e}");
                    sleep(Duration::from_secs(10));
                    continue;
                }
            }
        }
    }

    Ok(())
}

fn run<T: InverterDevice>(inverter: Inverter<T>) -> Result<(), Whatever> {
    let inverter_id = "powmr";
    let hass_device = hass::Device {
        name: "PowMR Inverter".to_string(),
        identifiers: vec![inverter_id.to_string()],
        manufacturer: "PowMR".to_string(),
        model: "PowMR 5000W DC 48V AC 220V All In One Inverter".to_string(),
    };
    let inverter_base_topic = format!(
        "homeassistant/sensor/{inverter_id}",
    );

    let mqtt_client = establish_mqtt_conn()?;
    create_entities(&mqtt_client, &hass_device, &inverter_base_topic)?;

    loop {
        let status1 = match inverter.status1() {
            Ok(status1) => status1,
            Err(e) => {
                println!("Error when getting device status: {e}");
                sleep(Duration::from_secs(10));
                continue;
            }
        };
        // println!("Device status: {status1:?}");
        for sensor in hass::SENSORS {
            let entity_name = sensor.entity_name;
            let entity_state = if let Some(state) = status1.entity_state(entity_name) {
                state
            } else {
                whatever!("Unknown entity name: {entity_name}");
            };
            let entity_state_msg = mqtt::Message::new(
                format!("{inverter_base_topic}/{entity_name}/state"),
                format!("{}", entity_state),
                0
            );
            if let Err(e) = mqtt_client.publish(entity_state_msg) {
                println!("Cannot publish entity state: {e}");
                break;
            }
        }
        sleep(Duration::from_secs(30));
    }
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        None => String::new(),
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
    }
}
