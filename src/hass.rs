use serde::Serialize;

pub const SENSORS: &'static [SensorConfig] = &[
    SensorConfig {
        entity_name: "grid_frequency",
        device_class: "voltage",
        unit_of_measurement: "V",
        icon: "mdi:power-plug",
    },
    SensorConfig {
        entity_name: "grid_frequency",
        device_class: "frequency",
        unit_of_measurement: "Hz",
        icon: "mdi:sine-wave",
    },
    SensorConfig {
        entity_name: "out_voltage",
        device_class: "voltage",
        unit_of_measurement: "V",
        icon: "mdi:power-plug",
    },
    SensorConfig {
        entity_name: "out_frequency",
        device_class: "frequency",
        unit_of_measurement: "Hz",
        icon: "mdi:sine-wave",
    },
    SensorConfig {
        entity_name: "load_watt",
        device_class: "power",
        unit_of_measurement: "W",
        icon: "mdi:lightning-bolt",
    },
    SensorConfig {
        entity_name: "load_percent",
        device_class: "power",
        unit_of_measurement: "%",
        icon: "mdi:lightning-bolt",
    },
    SensorConfig {
        entity_name: "battery_voltage",
        device_class: "voltage",
        unit_of_measurement: "V",
        icon: "mdi:battery-outline",
    },
    SensorConfig {
        entity_name: "battery_charge_current",
        device_class: "current",
        unit_of_measurement: "A",
        icon: "mdi:current-dc",
    },
    SensorConfig {
        entity_name: "battery_discharge_current",
        device_class: "current",
        unit_of_measurement: "A",
        icon: "mdi:current-dc",
    },
    SensorConfig {
        entity_name: "battery_capacity",
        device_class: "battery",
        unit_of_measurement: "%",
        icon: "mdi:battery-outline",
    },
    SensorConfig {
        entity_name: "heatsink_temperature",
        device_class: "Temperature",
        unit_of_measurement: "°C",
        icon: "mdi:thermometer",
    },
];

pub struct SensorConfig {
    pub entity_name: &'static str,
    pub device_class: &'static str,
    pub unit_of_measurement: &'static str,
    pub icon: &'static str,
}

// #[derive(Serialize)]
// struct Sensor {
//     field_name: String,
//     config_topic: String,
//     discovery: Discovery,
// }

#[derive(Serialize)]
pub struct Discovery {
    pub name: String,
    pub object_id: String,
    pub unique_id: String,
    pub state_topic: String,
    pub device: Device,
    pub device_class: String,
    pub unit_of_measurement: String,
    pub icon: String,
}

#[derive(Clone, Serialize)]
pub struct Device {
    pub name: String,
    pub identifiers: Vec<String>,
    pub manufacturer: String,
    pub model: String,
}
