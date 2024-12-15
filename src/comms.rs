use std::io::Write;

use cobs2::cobs;
use serial::prelude::*;

pub fn send(port: String, msg: &str, payload: Vec<u8>, index: Option<u8>) -> Result<u8, Box<dyn std::error::Error>> {
  let promise_index = index.unwrap_or(0);
  let packed = pack(msg.to_string(), payload, promise_index)?;

  let encoded = cobs::encode_vector(packed.as_slice())?;

  let mut port = serial::open(&port)?;

  port.reconfigure(&|settings| {
    settings.set_baud_rate(serial::BaudRate::Baud9600)?;
    Ok(())
  })?;

  port.write(encoded.as_slice())?;

  Ok(promise_index)
}

pub fn pack(msg: String, payload: Vec<u8>, index: u8) -> Result<Vec<u8>, String> {
  let mut buffer: Vec<u8> = vec!();

  if msg.len() > 255 {
    Err(format!("Message is too long ({}/255)", msg.len()))
  } else if payload.len() > 255 {
    Err(format!("Payload is too long ({}/255)", payload.len()))
  } else {
    buffer.push(msg.len().try_into().unwrap());
    buffer.extend_from_slice(msg.as_bytes());

    buffer.push(payload.len().try_into().unwrap());
    buffer.extend_from_slice(payload.as_slice());

    buffer.push(index);
    Ok(buffer)
  }
}

