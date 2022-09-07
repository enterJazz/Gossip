use bytes::Bytes;

pub struct Data {
    pub data_type: u16,
    pub data: Bytes,
}
