mod comms;
use clap::{Parser, Subcommand};

/// CLI for the Hack Club Blot
#[derive(Parser)]
#[command(version, about, long_about = None)]
struct Cli {
  #[command(subcommand)]
  command: Option<Commands>,

  #[arg(short, long)]
  port: String,
}

#[derive(Subcommand)]
enum Commands {
    /// Move the tool head to the specified coordinates
    Go {
        /// X coordinate
        x: f32,
        /// Y coordinate
        y: f32
    },
    MotorsOn,
    MotorsOff,
}

fn main() {
    let cli = Cli::parse();

    match &cli.command {
      Some(Commands::Go { x, y }) => {
        println!("Going to: ({}, {})", x, y);
        let mut payload: Vec<u8> = vec!();
        payload.extend_from_slice(x.to_ne_bytes().as_slice());
        payload.extend_from_slice(y.to_ne_bytes().as_slice());
        let send_result = comms::send(cli.port, "go", payload, None);

        if send_result.is_err() {
          panic!("Failed to send message to Blot: {}", send_result.unwrap_err());
        }

        println!("Message sent to Blot");
      }
      Some(Commands::MotorsOn) => {
        println!("Turning motors on");

        let send_result = comms::send(cli.port, "motorsOn", vec!(), None);

        if send_result.is_err() {
          panic!("Failed to send message to Blot: {}", send_result.unwrap_err());
        }

        println!("Message sent to Blot");
      }
      Some(Commands::MotorsOff) => {
        println!("Turning motors off");

        let send_result = comms::send(cli.port, "motorsOff", vec!(), None);

        if send_result.is_err() {
          panic!("Failed to send message to Blot: {}", send_result.unwrap_err());
        }

        println!("Message sent to Blot");
      }
      None => {}
  }
}