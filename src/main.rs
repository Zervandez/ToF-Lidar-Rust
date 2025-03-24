use serialport::{SerialPort, DataBits, Parity, StopBits, FlowControl};
use std::error::Error;
use std::io::{self, Write, Read, BufRead, BufReader};
use std::fs::OpenOptions;
use std::time::{Duration, Instant};
use std::thread;
use chrono::{Local, Timelike};
use serde_json::json;

/// Enum for switching between Binary or Text mode.
#[derive(Debug, Clone, Copy)]
enum Mode {
    Binary,
    Text,
}

/// **Change this variable to switch between Binary and Text mode**
const SENSOR_MODE: Mode = Mode::Text;

/// **Change this variable to control JSON file creation frequency (in minutes)**
const JSON_FILE_INTERVAL_MINUTES: u64 = 2; // Change to 5, 2, or any other value.
const PORTS: [&str; 3] = ["/dev/ttyACM1", "/dev/ttyACM0", "/dev/ttyACM2"];
const BAUD_RATE: u32 = 115200;
//const SAMPLING_INTERVAL_MS: u64 = 25; // 40 Hz sampling rate
const SAMPLING_INTERVAL_MS: u64 = 5; // 200 Hz sampling rate
const MAX_DISTANCE_MM: u16 = 6000; // Maximum valid distance in mm
const MIN_DISTANCE_MM: u16 = 500; // Minimum valid distance in mm
const TEXT_MODE_COMMAND: [u8; 4] = [0x00, 0x11, 0x01, 0x45];
const BINARY_MODE_COMMAND: [u8; 4] = [0x00, 0x11, 0x02, 0x4C];

/// Sends a command to the sensor to switch to Binary or Text mode.
fn send_command(port: &mut Box<dyn SerialPort>, command: &[u8]) -> io::Result<()> {
    println!("Sending mode switch command...");
    port.write_all(command)?;
    port.flush()?; // Ensure command is sent
    thread::sleep(Duration::from_millis(200)); // Allow time for processing
    Ok(())
}

/// Flushes the serial buffer before starting communication.
fn flush_serial(port: &mut Box<dyn SerialPort>) {
    let _ = port.clear(serialport::ClearBuffer::Input); // Flush input buffer
    let _ = port.clear(serialport::ClearBuffer::Output); // Flush output buffer
}

/// Saves sensor data to a JSON file, creating a new file based on the user-defined interval.
fn save_to_json(sensor_readings: &serde_json::Value, last_saved: &mut Instant) -> io::Result<()> {
    let now = Local::now();
    let rounded_minute = (now.minute() / JSON_FILE_INTERVAL_MINUTES as u32) * JSON_FILE_INTERVAL_MINUTES as u32;

    let filename = format!(
        "sensor_data_{}_{}-{:02}.json",
        now.format("%Y-%m-%d"),
        now.format("%H"),
        rounded_minute
    );

    // Create a new file only if the configured time interval has passed
    if last_saved.elapsed() >= Duration::from_secs(JSON_FILE_INTERVAL_MINUTES * 60) {
        *last_saved = Instant::now(); // Reset timer
    }

    let mut file = OpenOptions::new()
        .create(true) // Create if it doesn't exist
        .append(true) // Append if it already exists
        .open(filename)?;

    writeln!(file, "{}", sensor_readings.to_string())?;
    Ok(())
}

/// Reads a single distance value from a text-based sensor output.
fn read_text_distance(port: &mut Box<dyn SerialPort>) -> Option<u16> {
    let mut reader = BufReader::new(port);
    let mut line = String::new();

    match reader.read_line(&mut line) {
        Ok(n) if n > 0 => {
            if let Ok(distance) = line.trim().parse::<u16>() {
                if distance >= MIN_DISTANCE_MM && distance <= MAX_DISTANCE_MM {
                    return Some(distance);
                } else {
                    println!("WARNING: Ignoring invalid distance {} mm", distance);
                }
            }
        }
        _ => println!("WARNING: No valid text data received"),
    }
    None
}

fn main() -> Result<(), Box<dyn Error>> {
    let mut last_saved = Instant::now(); // Track the last save time
    let mut serial_ports: Vec<Option<Box<dyn SerialPort>>> = Vec::new();

    for &port_name in &PORTS {
        match serialport::new(port_name, BAUD_RATE)
            .timeout(Duration::from_millis(500)) // Increased timeout for reliability
            .data_bits(DataBits::Eight)
            .parity(Parity::None)
            .stop_bits(StopBits::One)
            .flow_control(FlowControl::None)
            .open()
        {
            Ok(mut p) => {
                println!("Opened serial port: {}", port_name);

                // **Flush serial buffer before using it**
                flush_serial(&mut p);

                // **Send mode switch command based on the selected mode**
                let command = match SENSOR_MODE {
                    Mode::Binary => BINARY_MODE_COMMAND,
                    Mode::Text => TEXT_MODE_COMMAND,
                };

                if let Err(e) = send_command(&mut p, &command) {
                    eprintln!("Failed to send command to {}: {}", port_name, e);
                }

                serial_ports.push(Some(p));
            }
            Err(e) => {
                eprintln!("Failed to open {}: {}", port_name, e);
                serial_ports.push(None);
            }
        }
    }

    if serial_ports.is_empty() {
        eprintln!("No serial ports available.");
        return Err(Box::new(io::Error::new(io::ErrorKind::Other, "No ports opened")));
    }

    loop {
        let timestamp = Local::now().format("%Y-%m-%d %H:%M:%S%.3f").to_string();
        let mut sensor_readings = json!({ "timestamp": timestamp, "sensors": {} });

        for (i, serial_port) in serial_ports.iter_mut().enumerate() {
            let mut final_distance = 0;

            if let Some(ref mut port) = serial_port {
                match SENSOR_MODE {
                    Mode::Text => {
                        // **Read a text-mode distance value**
                        if let Some(distance) = read_text_distance(port) {
                            final_distance = distance;
                            println!(
                                "[{}] {} | Parsed Distance: {} mm ({} cm)",
                                timestamp, PORTS[i], final_distance, final_distance / 10
                            );
                        }
                    }
                    Mode::Binary => {
                        let mut buffer = [0u8; 4];
                        if port.read_exact(&mut buffer).is_ok() && buffer[0] == 0x54 {
                            let distance = (buffer[2] as u16) << 8 | (buffer[3] as u16);
                            let distance = distance / 10; // Convert from 0.1mm to mm

                            if distance >= MIN_DISTANCE_MM && distance <= MAX_DISTANCE_MM {
                                final_distance = distance;
                                println!(
                                    "[{}] {} | Binary Distance: {} mm ({} cm)",
                                    timestamp, PORTS[i], final_distance, final_distance / 10
                                );
                            } else {
                                println!(
                                    "WARNING: Ignoring out-of-range distance {} mm from {}",
                                    distance, PORTS[i]
                                );
                            }
                        }
                    }
                }
            } else {
                println!("[{}] {} is unavailable (Setting distance to 0 mm)", timestamp, PORTS[i]);
            }

            sensor_readings["sensors"][PORTS[i]] = json!({
                "distance_mm": final_distance,
                "distance_cm": final_distance / 10
            });
        }

        // Save to JSON at user-defined intervals
        if let Err(e) = save_to_json(&sensor_readings, &mut last_saved) {
            eprintln!("Error saving to JSON: {}", e);
        }

        thread::sleep(Duration::from_millis(SAMPLING_INTERVAL_MS));
    }
}