use serde::Serialize;

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
