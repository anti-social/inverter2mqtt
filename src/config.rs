use serde::Deserialize;

#[derive(Deserialize, Debug)]
pub struct Config {
    pub inverter: InverterConfig,
    pub mqtt: MqttConfig,
}

#[derive(Deserialize, Debug)]
pub struct InverterConfig {
    pub id: String,
    pub name: String,
    pub manufacturer: String,
    pub model: String,
    pub usb: UsbConfig,
    pub commands: Vec<CommandConfig>,
}

#[derive(Deserialize, Debug)]
pub struct UsbConfig {
    pub vendor_id: u16,
    pub product_id: u16,
    pub interface: u8,
    pub request_params: RequestParams,
    pub response_params: ResponseParams,
}

#[derive(Deserialize, Debug, Clone)]
pub struct RequestParams {
    pub request_type: u8,
    pub request: u8,
    pub value: u16,
    pub index: u16,
    pub timeout_ms: u32,
}

#[derive(Deserialize, Debug, Clone)]
pub struct ResponseParams {
    pub endpoint: u8,
    pub timeout_ms: u32,
}

#[derive(Deserialize, Debug)]
pub struct CommandConfig {
    pub command: String,
    pub sensors: Vec<Option<SensorConfig>>,
}

#[derive(Deserialize, Debug)]
pub struct SensorConfig {
    pub name: String,
    pub human_name: Option<String>,
    pub value_type: ValueType,
    pub device_class: String,
    pub unit_of_measurement: String,
    pub icon: String,
}

#[derive(Deserialize, Debug)]
pub enum ValueType {
    #[serde(rename = "integer")]
    Integer,
    #[serde(rename = "float")]
    Float,
    #[serde(rename = "string")]
    String,
}

#[derive(Deserialize, Debug)]
pub struct MqttConfig {
    pub address: String,
    pub auth: Option<MqttAuth>,
}

#[derive(Deserialize, Debug)]
pub struct MqttAuth {
    pub user: String,
    pub password: String,
}
