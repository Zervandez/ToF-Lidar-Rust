use serialport::{SerialPort, DataBits, Parity, StopBits, FlowControl};
use std::error::Error;
use std::io::{self, Read, Write};
use std::fs::OpenOptions;
use std::time::{Instant};
use std::thread;
use std::time::Duration;
use chrono::Local; // For date and time handling
use serde_json::json; // JSON handling

const PORTS: [&str; 4] = ["/dev/ttyAMA10", "/dev/ttyACM1", "/dev/ttyACM0", "/dev/ttyACM2"];
const BAUD_RATE: u32 = 115200;

// Struct to hold serial port and last read time
struct SerialPortInfo {
    port: Option<Box<dyn SerialPort>>, // Now handles None for missing ports
    last_read_time: Option<Instant>,
}

// Convert two bytes to a decimal number
fn bytes_to_decimal(byte1: u8, byte2: u8) -> u16 {
    ((byte1 as u16) << 8) | (byte2 as u16)
}

// Get a human-readable timestamp: "YYYY-MM-DD HH:MM:SS.sss"
fn get_timestamp() -> String {
    Local::now().format("%Y-%m-%d %H:%M:%S%.3f").to_string()
}

// Get today's date for the JSON filename
fn get_today_date() -> String {
    Local::now().format("%Y-%m-%d").to_string()
}

// Generate today's JSON filename
fn get_json_filename() -> String {
    format!("sensor_data_{}.json", get_today_date())
}

// Save sensor data to a JSON file
fn save_to_json(sensor_readings: &serde_json::Value) -> io::Result<()> {
    let filename = get_json_filename();

    let mut file = OpenOptions::new()
        .create(true)
        .append(true) // Append mode
        .open(filename)?;

    writeln!(file, "{}", sensor_readings.to_string())?;
    Ok(())
}

fn main() -> Result<(), Box<dyn Error>> {
    let mut serial_ports = Vec::new();

    // Open and configure serial ports
    for &port_name in &PORTS {
        match serialport::new(port_name, BAUD_RATE)
            .timeout(Duration::from_millis(1)) // Minimize timeout for high-speed polling
            .data_bits(DataBits::Eight)
            .parity(Parity::None)
            .stop_bits(StopBits::One)
            .flow_control(FlowControl::None)
            .open()
        {
            Ok(p) => {
                println!("Opened serial port: {}", port_name);
                serial_ports.push(SerialPortInfo {
                    port: Some(p),
                    last_read_time: None,
                });
            }
            Err(e) => {
                eprintln!("Failed to open {}: {}", port_name, e);
                // Store None for unavailable ports
                serial_ports.push(SerialPortInfo {
                    port: None,
                    last_read_time: None,
                });
            }
        }
    }

    if serial_ports.is_empty() {
        eprintln!("No serial ports available.");
        return Err(Box::new(io::Error::new(io::ErrorKind::Other, "No ports opened")));
    }

    let mut buffer = [0u8; 256];

    // Continuous reading loop optimized for 240Hz
    loop {
        let timestamp = get_timestamp();
        let mut sensor_readings = json!({
            "timestamp": timestamp,
            "sensors": {}
        });

        for (i, serial_info) in serial_ports.iter_mut().enumerate() {
            let mut distance_mm = 0;
            let mut frequency = 0.0;

            if let Some(ref mut port) = serial_info.port {
                match port.read(&mut buffer) {
                    Ok(n) if n > 0 => {
                        let now = Instant::now();
                        frequency = serial_info.last_read_time.map_or(0.0, |last_time| {
                            1.0 / last_time.elapsed().as_secs_f64()
                        });
                        serial_info.last_read_time = Some(now); // Update last read time

                        // Print timestamp, port info, and frequency
                        println!(
                            "[{}] Received {} bytes from {} (Sampling Frequency: {:.2} Hz):",
                            timestamp, n, PORTS[i], frequency
                        );

                        // Process bytes in chunks of 4
                        for chunk in buffer[..n].chunks(4) {
                            print!("{}: ", PORTS[i]); // Print sensor name

                            for byte in chunk {
                                print!("{:02X} ", byte);
                            }

                            if chunk.len() == 4 {
                                distance_mm = bytes_to_decimal(chunk[2], chunk[3]);
                                println!(" | Distance: {} mm ({} cm)", distance_mm, distance_mm / 10);
                            }
                        }
                    }
                    Err(ref e) if e.kind() == io::ErrorKind::TimedOut => {
                        // No data received, default to 0
                        println!("[{}] No data from {} (Setting distance to 0 mm)", timestamp, PORTS[i]);
                    }
                    Err(e) => {
                        eprintln!("Error reading from {}: {}", PORTS[i], e);
                    }
                    _ => {}
                }
            } else {
                // If the port was never successfully opened, set distance to 0
                println!("[{}] {} is unavailable (Setting distance to 0 mm)", timestamp, PORTS[i]);
            }

            // Append sensor data to JSON object
            sensor_readings["sensors"][PORTS[i]] = json!({
                "distance_mm": distance_mm,
                "distance_cm": distance_mm / 10,
                "sampling_frequency_hz": frequency
            });
        }

        // Save data to JSON file
        if let Err(e) = save_to_json(&sensor_readings) {
            eprintln!("Error saving to JSON: {}", e);
        }

        // Reduce sleep time to maximize data reading rate
        thread::sleep(Duration::from_millis(2)); // Allows near 240 Hz polling
    }
}