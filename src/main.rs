use serialport::{SerialPort, DataBits, Parity, StopBits, FlowControl};
use std::error::Error;
use std::io::{self, Read, Write};
use std::fs::OpenOptions;
use std::time::{SystemTime, Instant, Duration};
use std::thread;
use chrono::Local;
use serde_json::json;

const PORTS: [&str; 4] = ["/dev/ttyAMA10", "/dev/ttyACM1", "/dev/ttyACM0", "/dev/ttyACM2"];
const BAUD_RATE: u32 = 115200;
const SAMPLING_INTERVAL_MS: u64 = 250; // 4 Hz sampling rate
const MAX_DISTANCE_MM: u16 = 4000; // **Now 4000 mm (4m) since values need /10 scaling**

// Function to convert two bytes to a decimal number (LSB First)
fn bytes_to_decimal(byte1: u8, byte2: u8) -> u16 {
    let raw_distance = (byte2 as u16) << 8 | (byte1 as u16);

    // **Scale down if necessary** (values seem to be in tenths of mm)
    let scaled_distance = raw_distance / 10;

    // **Ensure valid range**
    if scaled_distance > MAX_DISTANCE_MM {
        return 0;
    }

    scaled_distance
}

fn get_timestamp() -> String {
    Local::now().format("%Y-%m-%d %H:%M:%S%.3f").to_string()
}

fn save_to_json(sensor_readings: &serde_json::Value) -> io::Result<()> {
    let filename = format!("sensor_data_{}.json", Local::now().format("%Y-%m-%d"));
    let mut file = OpenOptions::new().create(true).append(true).open(filename)?;
    writeln!(file, "{}", sensor_readings.to_string())?;
    Ok(())
}

fn main() -> Result<(), Box<dyn Error>> {
    let mut serial_ports: Vec<Option<Box<dyn SerialPort>>> = Vec::new();

    for &port_name in &PORTS {
        match serialport::new(port_name, BAUD_RATE)
            .timeout(Duration::from_millis(100))
            .data_bits(DataBits::Eight)
            .parity(Parity::None)
            .stop_bits(StopBits::One)
            .flow_control(FlowControl::None)
            .open()
        {
            Ok(p) => {
                println!("Opened serial port: {}", port_name);
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

    let mut buffer = [0u8; 4];

    loop {
        let timestamp = get_timestamp();
        let mut sensor_readings = json!({ "timestamp": timestamp, "sensors": {} });

        for (i, serial_port) in serial_ports.iter_mut().enumerate() {
            let mut final_distance = 0;

            if let Some(ref mut port) = serial_port {
                match port.read_exact(&mut buffer) {
                    Ok(_) => {
                        // Debugging: Print raw bytes received
                        println!(
                            "[{}] {} RAW DATA: {:02X} {:02X} {:02X} {:02X}",
                            timestamp, PORTS[i], buffer[0], buffer[1], buffer[2], buffer[3]
                        );

                        // Ensure the first byte is a valid header (0x54)
                        match buffer[0] {
                            0x54 => {
                                final_distance = bytes_to_decimal(buffer[2], buffer[3]);

                                println!(
                                    "[{}] {} | Corrected Distance: {} mm ({} cm)",
                                    timestamp, PORTS[i], final_distance, final_distance / 10
                                );
                            }
                            _ => {
                                println!(
                                    "[{}] WARNING: Unexpected header byte {:#X} from {}, ignoring data",
                                    timestamp, buffer[0], PORTS[i]
                                );
                            }
                        }
                    }
                    Err(ref e) if e.kind() == io::ErrorKind::TimedOut => {
                        println!("[{}] No data from {} (Setting distance to 0 mm)", timestamp, PORTS[i]);
                    }
                    Err(e) => {
                        eprintln!("Error reading from {}: {}", PORTS[i], e);
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

        if let Err(e) = save_to_json(&sensor_readings) {
            eprintln!("Error saving to JSON: {}", e);
        }

        thread::sleep(Duration::from_millis(SAMPLING_INTERVAL_MS));
    }
}