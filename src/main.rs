mod comms;

use clap::{Parser, Subcommand};
use comms::{BlotPacket, PacketState};
use ringbuffer::{AllocRingBuffer, RingBuffer};
use std::{sync::Arc, time::Duration};
use tokio;
use tokio::sync::Mutex;
use uuid::Uuid;

/// CLI for the Hack Club Blot
#[derive(Parser)]
#[command(version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,

    #[arg(short, long)]
    port: String,
}

#[derive(Subcommand)]
enum Commands {
    /// Move the pen to the specified coordinates
    Go {
        /// X coordinate
        x: f32,
        /// Y coordinate
        y: f32,
    },
    /// Manage the Blot's stepper motors
    Motors {
        #[command(subcommand)]
        cmd: MotorsSubcommands,
    },
    /// Manage the Blot's origin
    Origin {
        #[command(subcommand)]
        cmd: OriginSubcommands,
    },
    /// Manage the Blot's pen
    Pen {
        #[command(subcommand)]
        cmd: PenSubcommands,
    },
}

#[derive(Subcommand)]
enum OriginSubcommands {
    /// Moves the pen towards the stored origin
    Move,
    /// Stores the current pen location as the Blot's origin
    Set,
}

#[derive(Subcommand)]
enum MotorsSubcommands {
    /// Turn the stepper motors on
    On,
    /// Turn the stepper motors off
    Off,
}

#[derive(Subcommand)]
enum PenSubcommands {
    /// Move the pen up
    Up,
    /// Move the pen down
    Down,
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();

    let packet_queue = Arc::new(Mutex::new(AllocRingBuffer::new(10)));
    let comms_thread = tokio::spawn(comms::initialize(cli.port, packet_queue.clone()));

    let mut packets = packet_queue.lock().await;

    match &cli.command {
        Commands::Go { x, y } => {
            println!("Going to: ({}, {})", x, y);
            let mut payload: Vec<u8> = vec![];
            payload.extend_from_slice(x.to_ne_bytes().as_slice());
            payload.extend_from_slice(y.to_ne_bytes().as_slice());

            let id = Uuid::new_v4();
            packets.push(BlotPacket {
                id,
                msg: "go".to_string(),
                payload: [x.to_le_bytes(), y.to_le_bytes()].concat(),
                index: None,
                state: comms::PacketState::Queued,
            });

            // Drop mutex so comms thread can gain a lock
            std::mem::drop(packets);

            wait_for_ack(packet_queue, id).await;
        }
        Commands::Motors { cmd } => {
            match cmd {
                MotorsSubcommands::On => {
                    println!("Turning stepper motors on");

                    let id = Uuid::new_v4();
                    packets.push(BlotPacket {
                        id,
                        msg: "motorsOn".to_string(),
                        payload: vec![],
                        index: None,
                        state: comms::PacketState::Queued,
                    });

                    // Drop mutex so comms thread can gain a lock
                    std::mem::drop(packets);

                    wait_for_ack(packet_queue, id).await;
                }
                MotorsSubcommands::Off => {
                    println!("Turning stepper motors off");

                    let id = Uuid::new_v4();
                    packets.push(BlotPacket {
                        id,
                        msg: "motorsOff".to_string(),
                        payload: vec![],
                        index: None,
                        state: comms::PacketState::Queued,
                    });

                    // Drop mutex so comms thread can gain a lock
                    std::mem::drop(packets);
                    wait_for_ack(packet_queue, id).await;
                }
            }
        }
        Commands::Origin { cmd } => {
            match cmd {
                OriginSubcommands::Move => {
                    println!("Moving towards origin");

                    let id = Uuid::new_v4();
                    packets.push(BlotPacket {
                        id,
                        msg: "moveTowardsOrigin".to_string(),
                        payload: vec![],
                        index: None,
                        state: comms::PacketState::Queued,
                    });

                    // Drop mutex so comms thread can gain a lock
                    std::mem::drop(packets);
                    wait_for_ack(packet_queue, id).await;
                }
                OriginSubcommands::Set => {
                    println!("Setting origin");

                    let id = Uuid::new_v4();
                    packets.push(BlotPacket {
                        id,
                        msg: "setOrigin".to_string(),
                        payload: vec![],
                        index: None,
                        state: comms::PacketState::Queued,
                    });

                    // Drop mutex so comms thread can gain a lock
                    std::mem::drop(packets);
                    wait_for_ack(packet_queue, id).await;
                }
            }
        }
        Commands::Pen { cmd } => {
            match cmd {
                PenSubcommands::Up => {
                    println!("Moving pen up");

                    let id = Uuid::new_v4();
                    packets.push(BlotPacket {
                        id,
                        msg: "servo".to_string(),
                        payload: 1000_u32.to_le_bytes().to_vec(),
                        index: None,
                        state: comms::PacketState::Queued,
                    });

                    // Drop mutex so comms thread can gain a lock
                    std::mem::drop(packets);
                    wait_for_ack(packet_queue, id).await;
                }
                PenSubcommands::Down => {
                    println!("Moving pen down");

                    let id = Uuid::new_v4();
                    packets.push(BlotPacket {
                        id,
                        msg: "servo".to_string(),
                        payload: 1700_u32.to_le_bytes().to_vec(),
                        index: None,
                        state: comms::PacketState::Queued,
                    });

                    // Drop mutex so comms thread can gain a lock
                    std::mem::drop(packets);
                    wait_for_ack(packet_queue, id).await;
                }
            }
        }
    }

    comms_thread.abort();
}

async fn wait_for_ack(packet_queue: Arc<Mutex<AllocRingBuffer<BlotPacket>>>, id: Uuid) {
    loop {
        let packets = packet_queue.lock().await;

        let packet_result = packets.iter().find(|p| p.id == id);

        if let Some(packet) = packet_result {
            if packet.state == PacketState::Resolved {
                break;
            }
        }

        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}
