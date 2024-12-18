use std::io::{Read, Write};
use std::str;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;

use cobs2::cobs;
use ringbuffer::{AllocRingBuffer, RingBuffer};
use serialport;
use uuid::Uuid;

#[derive(Clone, PartialEq)]
pub enum PacketState {
    Queued,
    Sent,
    Resolved,
    Received,
}

#[derive(Clone)]
pub struct BlotPacket {
    pub id: Uuid,
    pub msg: String,
    pub payload: Vec<u8>,
    pub index: Option<u8>,
    pub state: PacketState,
}

pub async fn initialize(port: String, packet_queue: Arc<Mutex<AllocRingBuffer<BlotPacket>>>) {
    let mut comms = BlotComms::initialize(port).expect("Failed to initialize comms");

    loop {
        let packet_result = comms.read();
        let mut packets = packet_queue.lock().await;

        match packet_result {
            Ok(packet) => match packet.msg.as_str() {
                "ack" => {
                    let sent_packet = packets
                        .iter_mut()
                        .find(|p| p.index == packet.index)
                        .expect("Received an ack for a nonexistent message");

                    sent_packet.state = PacketState::Resolved;
                }
                _ => {
                    panic!("Unexpected packet type: {}", packet.msg)
                }
            },
            Err(_) => {
                let mut index = match packets.to_vec().last() {
                    Some(p) => p.index.unwrap_or(0),
                    None => 0,
                };
                let mut to_send: Vec<&mut BlotPacket> = packets
                    .iter_mut()
                    .filter(|p| p.state == PacketState::Queued)
                    .collect();

                for packet in to_send.iter_mut() {
                    packet.index = Some(index);
                    index = (index + 1) % 9;
                    comms.send(*packet).await.expect("Failed to send message");
                    packet.state = PacketState::Sent;
                }
            }
        }
    }
}

pub struct BlotComms {
    port: Box<dyn serialport::SerialPort>,
}

impl BlotComms {
    fn initialize(port: String) -> Result<BlotComms, serialport::Error> {
        let port = serialport::new(&port, 9600)
            .timeout(Duration::from_millis(10))
            .open()?;

        Ok(BlotComms { port })
    }

    fn read(&mut self) -> Result<BlotPacket, std::str::Utf8Error> {
        let mut response: Vec<u8> = vec![];

        // 0x0a (LF) terminates each message from the Blot
        while response.last() != Some(&0x0a) {
            let mut data: Vec<u8> = vec![];
            let result = self.port.read(data.as_mut_slice());
            if result.is_err() {
                break;
            }

            response.extend_from_slice(&data);
        }

        let unpacked = Self::unpack(&response)?;

        Ok(unpacked)
    }

    async fn send(&mut self, packet: &BlotPacket) -> Result<u8, Box<dyn std::error::Error>> {
        let packed = Self::pack(&packet)?;

        let encoded = cobs::encode_vector(&packed)?;

        self.port.write(&encoded)?;

        let mut response: Vec<u8> = vec![];
        // 0x0a (LF) terminates each message from the Blot
        while response.last() != Some(&0x0a) {
            let mut data: Vec<u8> = vec![];
            let size = self.port.read(data.as_mut_slice())?;

            if size == 0 {
                break;
            }

            response.extend_from_slice(&data);
        }

        Ok(packet.index.unwrap())
    }

    fn pack(packet: &BlotPacket) -> Result<Vec<u8>, String> {
        let mut buffer: Vec<u8> = vec![];

        if packet.msg.len() > 255 {
            Err(format!("Message is too long ({}/255)", packet.msg.len()))
        } else if packet.payload.len() > 255 {
            Err(format!(
                "Payload is too long ({}/255)",
                packet.payload.len()
            ))
        } else {
            buffer.push(packet.msg.len().try_into().unwrap());
            buffer.extend_from_slice(packet.msg.as_bytes());

            buffer.push(packet.payload.len().try_into().unwrap());
            buffer.extend_from_slice(&packet.payload);

            buffer.push(packet.index.expect("No index on packed packet"));
            Ok(buffer)
        }
    }

    fn unpack(buf: &[u8]) -> Result<BlotPacket, std::str::Utf8Error> {
        let msg_length = buf[0];
        let mut msg_bytes: Vec<u8> = vec![];
        for n in 1..msg_length {
            msg_bytes.push(buf[n as usize]);
        }
        let msg = str::from_utf8(&msg_bytes)?.to_string();

        let payload_length = buf[msg_length as usize];
        let mut payload_bytes: Vec<u8> = vec![];
        for n in (msg_length + 1)..(msg_length + payload_length) {
            payload_bytes.push(buf[n as usize]);
        }
        let payload = payload_bytes;

        let index = Some(buf[(msg_length + payload_length) as usize]);

        Ok(BlotPacket {
            id: Uuid::new_v4(),
            msg,
            payload,
            index,
            state: PacketState::Received,
        })
    }
}
