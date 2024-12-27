mod comms;

use clap::{Parser, Subcommand};
use comms::{BlotPacket, PacketState};
use crossterm::{
    event::{self, DisableMouseCapture, Event as CEvent, KeyCode, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, LeaveAlternateScreen},
};
use futures::{task::noop_waker_ref, FutureExt};
use inquire::{self, Select};
use ringbuffer::{AllocRingBuffer, RingBuffer};
use serialport::{self, SerialPortType};
use std::{
    future::Future,
    io::{self, Stdout},
    panic,
    pin::Pin,
    process,
    sync::{mpsc, Arc},
    task::{Context, Poll},
    thread,
    time::{Duration, Instant},
};
use tokio::{self, sync::Mutex};
use tui::{
    backend::CrosstermBackend,
    layout::{Alignment, Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Span, Spans},
    widgets::{Block, BorderType, Borders, Paragraph, Tabs},
    Terminal,
};
use uuid::Uuid;

/// CLI for the Hack Club Blot
#[derive(Parser)]
#[command(version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,

    #[arg(short, long)]
    port: Option<String>,
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
    /// Enter interactive mode
    Interactive,
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

enum Event<I> {
    Input(I),
    Tick,
}

#[derive(PartialEq)]
enum InteractiveDestination {
    Coordinates(InteractiveCoordinates),
    Direction(InteractiveDirection),
}

#[derive(PartialEq)]
struct InteractiveCoordinates {
    x: f32,
    y: f32,
}

#[derive(PartialEq)]
enum InteractiveDirection {
    Forward,
    Back,
    Left,
    Right,
}

#[derive(PartialEq)]
enum InteractivePosStatus {
    Initializing,
    Moving(InteractiveDestination),
    Stopped,
}

enum InteractivePenStatus {
    Up,
    Down,
}

#[derive(PartialEq)]
enum InteractiveEditStatus {
    StepSize,
    GoCoordinates,
    None,
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    let port = match cli.port {
        Some(p) => p,
        None => {
            let ports = serialport::available_ports().unwrap_or(vec![]);

            let filtered = ports
                .iter()
                .filter(|p| match p.port_type {
                    SerialPortType::UsbPort(_) => true,
                    _ => false,
                })
                .collect::<Vec<_>>();

            if filtered.len() == 0 {
                println!("No USB serial ports available on system. Make sure the Blot is powered on and plugged in via USB.");
                process::exit(1);
            }

            let options = filtered
                .iter()
                .map(|p| p.port_name.clone())
                .collect::<Vec<_>>();

            let ans = Select::new("Choose a serial port", options).prompt();

            match ans {
                Ok(choice) => choice,
                Err(_) => {
                    println!("Could not determine port to use");
                    process::exit(1);
                }
            }
        }
    };

    let packet_queue = Arc::new(Mutex::new(AllocRingBuffer::new(10)));
    let comms_thread = tokio::spawn(comms::initialize(port, packet_queue.clone()));

    // Exit main thread if comms thread panics
    let orig_hook = panic::take_hook();
    panic::set_hook(Box::new(move |panic_info| {
        orig_hook(panic_info);
        process::exit(1);
    }));

    match &cli.command {
        Commands::Go { x, y } => {
            println!("Going to: ({}, {})", x, y);
            send_command(
                packet_queue,
                "go",
                [x.to_le_bytes(), y.to_le_bytes()].concat(),
            )
            .await;
        }
        Commands::Motors { cmd } => match cmd {
            MotorsSubcommands::On => {
                println!("Turning stepper motors on");
                send_command(packet_queue.clone(), "motorsOn", vec![]).await;
                send_command(packet_queue.clone(), "motorsOn", vec![]).await;
                send_command(packet_queue.clone(), "motorsOn", vec![]).await;
                send_command(packet_queue.clone(), "motorsOn", vec![]).await;
                send_command(packet_queue.clone(), "motorsOn", vec![]).await;
                send_command(packet_queue.clone(), "motorsOn", vec![]).await;
                send_command(packet_queue.clone(), "motorsOn", vec![]).await;
                send_command(packet_queue.clone(), "motorsOn", vec![]).await;
                send_command(packet_queue.clone(), "motorsOn", vec![]).await;
                send_command(packet_queue.clone(), "motorsOn", vec![]).await;
            }
            MotorsSubcommands::Off => {
                println!("Turning stepper motors off");
                send_command(packet_queue, "motorsOff", vec![]).await;
            }
        },
        Commands::Origin { cmd } => match cmd {
            OriginSubcommands::Move => {
                println!("Moving towards origin");
                send_command(packet_queue, "moveTowardsOrigin", vec![]).await;
            }
            OriginSubcommands::Set => {
                println!("Setting origin");
                send_command(packet_queue, "setOrigin", vec![]).await;
            }
        },
        Commands::Pen { cmd } => match cmd {
            PenSubcommands::Up => {
                println!("Moving pen up");
                send_command(packet_queue, "servo", 1000_u32.to_le_bytes().to_vec()).await;
            }
            PenSubcommands::Down => {
                println!("Moving pen down");
                send_command(packet_queue, "servo", 1700_u32.to_le_bytes().to_vec()).await;
            }
        },
        Commands::Interactive => {
            let orig_hook = panic::take_hook();
            panic::set_hook(Box::new(move |panic_info| {
                let stdout = io::stdout();
                let backend = CrosstermBackend::new(stdout);
                let terminal = Terminal::new(backend).expect("Failed to initialize tui backend");
                restore_terminal(terminal);

                orig_hook(panic_info);
            }));

            let (tx, rx) = mpsc::channel();
            let tick_rate = Duration::from_millis(200);
            thread::spawn(move || {
                let mut last_tick = Instant::now();
                loop {
                    let timeout = tick_rate
                        .checked_sub(last_tick.elapsed())
                        .unwrap_or_else(|| Duration::from_secs(0));

                    if event::poll(timeout).expect("poll works") {
                        if let CEvent::Key(key) = event::read().expect("can read events") {
                            tx.send(Event::Input(key)).expect("can send events");
                        }
                    }

                    if last_tick.elapsed() >= tick_rate {
                        if let Ok(_) = tx.send(Event::Tick) {
                            last_tick = Instant::now();
                        }
                    }
                }
            });

            let stdout = io::stdout();
            let backend = CrosstermBackend::new(stdout);
            let mut terminal = Terminal::new(backend).expect("Failed to initialize tui backend");
            terminal.clear().expect("Failed to clear terminal");
            enable_raw_mode().expect("failed to enable raw mode");

            let menu_titles = vec![
                "Go",
                "Forward",
                "Back",
                "Left",
                "Right",
                "Up",
                "Pen down",
                "Change Step",
                "Quit",
            ];
            let mut interactive_pos_status = InteractivePosStatus::Initializing;
            let mut interactive_pen_status = InteractivePenStatus::Up;
            let mut interactive_coordinates = InteractiveCoordinates { x: 0.0, y: 0.0 };
            let mut interactive_edit_status = InteractiveEditStatus::None;

            let mut edit_text = "".to_string();
            let mut step_size = 5_f32;

            let mut pending_futures: Vec<Pin<Box<dyn Future<Output = BlotPacket>>>> = vec![];

            let mut ctx = Context::from_waker(noop_waker_ref());
            loop {
                pending_futures = pending_futures
                    .into_iter()
                    .filter_map(|mut future| {
                        let res = (&mut future).poll_unpin(&mut ctx);

                        match res {
                            Poll::Ready(p) => match p.msg.as_str() {
                                "go" => {
                                    interactive_pos_status = InteractivePosStatus::Stopped;
                                    interactive_coordinates = InteractiveCoordinates {
                                        x: f32::from_le_bytes(p.payload[0..4].try_into().unwrap()),
                                        y: f32::from_le_bytes(p.payload[4..8].try_into().unwrap()),
                                    };

                                    None
                                }
                                "servo" => {
                                    let servo_position =
                                        u32::from_le_bytes(p.payload[0..4].try_into().unwrap());
                                    interactive_pen_status = if servo_position == 1700 {
                                        InteractivePenStatus::Down
                                    } else {
                                        InteractivePenStatus::Up
                                    };

                                    None
                                }
                                _ => None,
                            },
                            Poll::Pending => Some(future),
                        }
                    })
                    .collect();

                terminal
                    .draw(|f| {
                        let main_chunks = Layout::default()
                            .direction(Direction::Vertical)
                            .margin(2)
                            .constraints(
                                [
                                    Constraint::Length(3),
                                    Constraint::Length(4),
                                    Constraint::Min(2),
                                    Constraint::Length(3),
                                ]
                                .as_ref(),
                            )
                            .split(f.size());

                        let status_chunks = Layout::default()
                            .direction(Direction::Horizontal)
                            .margin(0)
                            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
                            .split(main_chunks[1]);

                        let info =
                            Paragraph::new("Blot CLI - made by Samuel Fernandez (@polypixeldev)")
                                .style(Style::default().fg(Color::LightCyan))
                                .alignment(Alignment::Center)
                                .block(
                                    Block::default()
                                        .borders(Borders::ALL)
                                        .style(Style::default().fg(Color::White))
                                        .border_type(BorderType::Plain),
                                );

                        let menu: Vec<_> = menu_titles
                            .iter()
                            .map(|t| {
                                let (first, rest) = t.split_at(1);
                                Spans::from(vec![
                                    Span::styled(
                                        first,
                                        Style::default()
                                            .fg(Color::Yellow)
                                            .add_modifier(Modifier::UNDERLINED),
                                    ),
                                    Span::styled(rest, Style::default().fg(Color::White)),
                                ])
                            })
                            .collect();

                        let tabs = Tabs::new(menu)
                            .block(Block::default().title("Controls").borders(Borders::ALL))
                            .style(Style::default().fg(Color::White))
                            .highlight_style(Style::default().fg(Color::Yellow))
                            .divider(Span::raw("|"));

                        let pos_text = match &interactive_pos_status {
                            InteractivePosStatus::Initializing => {
                                "Initializating Blot interactive mode"
                            }
                            InteractivePosStatus::Moving(destination) => {
                                let destination_text = match &destination {
                                    InteractiveDestination::Direction(dir) => match &dir {
                                        InteractiveDirection::Forward => "forwards",
                                        InteractiveDirection::Back => "backwards",
                                        InteractiveDirection::Left => "left",
                                        InteractiveDirection::Right => "right",
                                    },
                                    InteractiveDestination::Coordinates(coordinates) => {
                                        &format!("to ({}, {})", coordinates.x, coordinates.y)
                                    }
                                };

                                &format!("Blot is moving {destination_text}")
                            }
                            InteractivePosStatus::Stopped => &format!(
                                "Blot is stopped at ({}, {})",
                                interactive_coordinates.x, interactive_coordinates.y
                            ),
                        };
                        let pen_text = match &interactive_pen_status {
                            InteractivePenStatus::Down => "Pen is DOWN",
                            InteractivePenStatus::Up => "Pen is UP",
                        };
                        let status_text = format!("{pos_text}\n{pen_text}");

                        let blot_status = Paragraph::new(status_text)
                            .style(Style::default().fg(Color::LightGreen))
                            .alignment(Alignment::Left)
                            .block(
                                Block::default()
                                    .borders(Borders::all().difference(Borders::RIGHT))
                                    .style(Style::default().fg(Color::White))
                                    .title("Status")
                                    .border_type(BorderType::Plain),
                            );

                        let edit_type_text = match &interactive_edit_status {
                            InteractiveEditStatus::GoCoordinates => "Coordinates (x,y): ",
                            InteractiveEditStatus::StepSize => "Step size: ",
                            InteractiveEditStatus::None => "",
                        };
                        let edit_text = format!("{edit_type_text}{edit_text}");

                        let input_box = Paragraph::new(edit_text)
                            .style(Style::default().fg(Color::Yellow))
                            .alignment(Alignment::Right)
                            .block(
                                Block::default()
                                    .borders(Borders::all().difference(Borders::LEFT))
                                    .style(Style::default().fg(Color::White))
                                    .border_type(BorderType::Plain),
                            );

                        f.render_widget(info, main_chunks[0]);
                        f.render_widget(blot_status, status_chunks[0]);
                        f.render_widget(input_box, status_chunks[1]);
                        f.render_widget(tabs, main_chunks[3]);
                    })
                    .expect("Failed to draw tui");

                if interactive_pos_status == InteractivePosStatus::Initializing {
                    send_command(
                        packet_queue.clone(),
                        "servo",
                        1000_u32.to_le_bytes().to_vec(),
                    )
                    .await;
                    send_command(packet_queue.clone(), "motorsOn", vec![]).await;
                    send_command(packet_queue.clone(), "go", vec![0, 0, 0, 0, 0, 0, 0, 0]).await;
                    interactive_pos_status = InteractivePosStatus::Stopped;
                }

                if interactive_edit_status != InteractiveEditStatus::None {
                    match rx.recv() {
                        Ok(Event::Input(event)) => {
                            match event.code {
                                KeyCode::Char('c') => {
                                    if event.modifiers.contains(KeyModifiers::CONTROL) {
                                        restore_terminal(terminal);
                                        break;
                                    }
                                }
                                KeyCode::Char(c) => {
                                    let num_parse = c.to_string().parse::<f32>();

                                    if num_parse.is_err() && c != '.' && c != ',' {
                                        continue;
                                    }

                                    let new_edit_text = format!("{edit_text}{c}");
                                    edit_text = new_edit_text;
                                }
                                KeyCode::Backspace | KeyCode::Delete => {
                                    edit_text = edit_text[0..(edit_text.len() - 1)].to_string();
                                }
                                KeyCode::Enter => {
                                    match &interactive_edit_status {
                                        InteractiveEditStatus::GoCoordinates => {
                                            let split = edit_text.split(",").collect::<Vec<_>>();
                                            let x_parse = split[0].trim().parse::<f32>();
                                            let y_parse = split[1].trim().parse::<f32>();

                                            if x_parse.is_err() | y_parse.is_err() {
                                                continue;
                                            }

                                            let mut new_x = x_parse.unwrap();
                                            let mut new_y = y_parse.unwrap();

                                            if new_y < 0.0 {
                                                new_y = 0.0;
                                            }
                                            if new_y > 125.0 {
                                                new_y = 125.0;
                                            }
                                            if new_x < 0.0 {
                                                new_x = 0.0;
                                            }
                                            if new_x > 125.0 {
                                                new_x = 125.0;
                                            }

                                            let command_future = send_command(
                                                packet_queue.clone(),
                                                "go",
                                                [new_x.to_le_bytes(), new_y.to_le_bytes()].concat(),
                                            );
                                            interactive_pos_status = InteractivePosStatus::Moving(
                                                InteractiveDestination::Coordinates(
                                                    InteractiveCoordinates { x: new_x, y: new_y },
                                                ),
                                            );
                                            pending_futures.push(Box::pin(command_future));
                                        }
                                        InteractiveEditStatus::StepSize => {
                                            let step_parse = edit_text.trim().parse::<f32>();

                                            if step_parse.is_err() {
                                                continue;
                                            }

                                            let new_step_size = step_parse.unwrap();

                                            if (new_step_size <= 0.0) | (new_step_size >= 125.0) {
                                                continue;
                                            }

                                            step_size = new_step_size;
                                        }
                                        _ => {}
                                    }
                                    interactive_edit_status = InteractiveEditStatus::None;
                                    edit_text = "".to_string();
                                }
                                _ => {}
                            };
                        }
                        Ok(Event::Tick) => {}
                        Err(_) => {}
                    }
                } else {
                    match rx.recv() {
                        Ok(Event::Input(event)) => match event.code {
                            KeyCode::Char('q') => {
                                restore_terminal(terminal);
                                break;
                            }
                            KeyCode::Char('g') => {
                                interactive_edit_status = InteractiveEditStatus::GoCoordinates;
                            }
                            KeyCode::Char('c') => {
                                if event.modifiers.contains(KeyModifiers::CONTROL) {
                                    restore_terminal(terminal);
                                    break;
                                }
                                interactive_edit_status = InteractiveEditStatus::StepSize;
                            }
                            KeyCode::Char('f') | KeyCode::Char('w') => {
                                let mut new_y = interactive_coordinates.y + step_size;
                                if new_y < 0.0 {
                                    new_y = 0.0;
                                }
                                if new_y > 125.0 {
                                    new_y = 125.0;
                                }
                                let command_future = send_command(
                                    packet_queue.clone(),
                                    "go",
                                    [interactive_coordinates.x.to_le_bytes(), new_y.to_le_bytes()]
                                        .concat(),
                                );
                                interactive_pos_status = InteractivePosStatus::Moving(
                                    InteractiveDestination::Direction(
                                        InteractiveDirection::Forward,
                                    ),
                                );
                                pending_futures.push(Box::pin(command_future));
                            }
                            KeyCode::Char('a') | KeyCode::Char('l') => {
                                let mut new_x = interactive_coordinates.x - step_size;
                                if new_x < 0.0 {
                                    new_x = 0.0;
                                }
                                if new_x > 125.0 {
                                    new_x = 125.0;
                                }
                                let command_future = send_command(
                                    packet_queue.clone(),
                                    "go",
                                    [new_x.to_le_bytes(), interactive_coordinates.y.to_le_bytes()]
                                        .concat(),
                                );
                                interactive_pos_status = InteractivePosStatus::Moving(
                                    InteractiveDestination::Direction(InteractiveDirection::Left),
                                );
                                pending_futures.push(Box::pin(command_future));
                            }
                            KeyCode::Char('b') | KeyCode::Char('s') => {
                                let mut new_y = interactive_coordinates.y - step_size;
                                if new_y < 0.0 {
                                    new_y = 0.0;
                                }
                                if new_y > 125.0 {
                                    new_y = 125.0;
                                }
                                let command_future = send_command(
                                    packet_queue.clone(),
                                    "go",
                                    [interactive_coordinates.x.to_le_bytes(), new_y.to_le_bytes()]
                                        .concat(),
                                );
                                interactive_pos_status = InteractivePosStatus::Moving(
                                    InteractiveDestination::Direction(InteractiveDirection::Back),
                                );
                                pending_futures.push(Box::pin(command_future));
                            }
                            KeyCode::Char('r') | KeyCode::Char('d') => {
                                let mut new_x = interactive_coordinates.x + step_size;
                                if new_x < 0.0 {
                                    new_x = 0.0;
                                }
                                if new_x > 125.0 {
                                    new_x = 125.0;
                                }
                                let command_future = send_command(
                                    packet_queue.clone(),
                                    "go",
                                    [new_x.to_le_bytes(), interactive_coordinates.y.to_le_bytes()]
                                        .concat(),
                                );
                                interactive_pos_status = InteractivePosStatus::Moving(
                                    InteractiveDestination::Direction(InteractiveDirection::Right),
                                );
                                pending_futures.push(Box::pin(command_future));
                            }
                            KeyCode::Char('u') | KeyCode::Up => {
                                let command_future = send_command(
                                    packet_queue.clone(),
                                    "servo",
                                    1000_u32.to_le_bytes().to_vec(),
                                );
                                interactive_pen_status = InteractivePenStatus::Up;
                                pending_futures.push(Box::pin(command_future));
                            }
                            KeyCode::Char('p') | KeyCode::Down => {
                                let command_future = send_command(
                                    packet_queue.clone(),
                                    "servo",
                                    1700_u32.to_le_bytes().to_vec(),
                                );
                                interactive_pen_status = InteractivePenStatus::Down;
                                pending_futures.push(Box::pin(command_future));
                            }
                            _ => {}
                        },
                        Ok(Event::Tick) => {}
                        Err(_) => {}
                    }
                }
            }
        }
    }

    comms_thread.abort();
}

fn restore_terminal(mut terminal: Terminal<CrosstermBackend<Stdout>>) {
    disable_raw_mode().expect("Failed to restore terminal");
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )
    .expect("Failed to restore terminal");
    terminal.clear().expect("Failed to clear terminal");
    terminal.show_cursor().expect("Failed to restore terminal");
}

async fn send_command(
    packet_queue: Arc<Mutex<AllocRingBuffer<BlotPacket>>>,
    msg: &str,
    payload: Vec<u8>,
) -> BlotPacket {
    let mut packets = packet_queue.lock().await;

    let id = Uuid::new_v4();
    let packet = BlotPacket {
        id,
        msg: msg.to_string(),
        payload,
        index: None,
        state: comms::PacketState::Queued,
    };
    packets.push(packet.clone());

    // Drop mutex so comms thread can gain a lock
    std::mem::drop(packets);
    wait_for_ack(packet_queue, id).await;

    packet
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
