use core::clone::Clone;
use core::marker::Copy;
use core::option::Option::{self, None, Some};
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::pubsub::PubSubChannel;
use zerocopy::{
    byteorder::big_endian::{I16, U16},
    FromBytes,
};

#[derive(Clone, Copy)]
pub enum DisplayMessage {
    ActiveData(Sen55Frame),
    SystemError(ErrorMeta),
}

#[derive(Clone, Copy)]
pub struct ErrorMeta {
    pub code: &'static str,
    pub long_message: &'static str,
}

pub static DATA_BUS: PubSubChannel<CriticalSectionRawMutex, DisplayMessage, 8, 2, 1> =
    PubSubChannel::new();

#[derive(FromBytes, Clone, Copy)]
#[repr(C, packed)]
pub struct Sen55Frame {
    // PM values are in 0.1 µg/m³ (divide by 10 for actual value)
    pub pm1_0: U16,
    pub pm1_0_crc: u8,
    pub pm2_5: U16,
    pub pm2_5_crc: u8,
    pub pm4_0: U16,
    pub pm4_0_crc: u8,
    pub pm10_0: U16,
    pub pm10_0_crc: u8,

    // Environmental values
    pub humidity: I16, // in 0.01 %RH
    pub humidity_crc: u8,
    pub temperature: I16, // in 0.005 °C
    pub temperature_crc: u8,
    pub voc_index: I16, // in 0.1 units
    pub voc_crc: u8,
    pub nox_index: I16, // in 0.1 units
    pub nox_crc: u8,
}

impl Sen55Frame {
    /// Validates all 8 CRC bytes in the frame
    pub fn parse(buffer: &[u8; 24]) -> Option<Self> {
        let frame = Self::read_from_bytes(buffer).ok()?;

        // Sensirion transmits in segments: [MSB, LSB, CRC]
        // validate each triplet.
        for chunk in buffer.chunks_exact(3) {
            if !validate_crc(&chunk[0..2], chunk[2]) {
                return None;
            }
        }

        Some(frame)
    }
}

/// Sensirion CRC8 calculation. Frankly, can probably just save this for destination
fn validate_crc(data: &[u8], expected: u8) -> bool {
    let mut crc: u8 = 0xFF;
    for &byte in data {
        crc ^= byte;
        for _ in 0..8 {
            if (crc & 0x80) != 0 {
                crc = (crc << 1) ^ 0x31;
            } else {
                crc <<= 1;
            }
        }
    }
    crc == expected
}

#[repr(packed)]
pub struct Sen55Packed {
    pub data: [u8; 24],
    pub sen_id: u32,
}

impl Sen55Packed {
    pub fn new(frame: &Sen55Frame) -> Self {
        // This copies the entire memory layout of the struct at once.
        // Since it is #[repr(C, packed)], it is guaranteed to be
        // a 24-byte contiguous block in memory.
        let data: [u8; 24] = unsafe { core::mem::transmute_copy(frame) };

        Self {
            data,
            sen_id: crate::secrets::SEN_ID,
        }
    }
}
