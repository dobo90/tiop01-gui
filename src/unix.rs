use crate::thermal::PortOpener;

use anyhow::anyhow;
use serialport::SerialPort;
use std::{io, marker::PhantomData, time::Duration};

pub struct SerialPortOpener<'a> {
    phantom: PhantomData<&'a ()>,
}

impl<'a> SerialPortOpener<'a> {
    pub fn new() -> Self {
        Self {
            phantom: PhantomData,
        }
    }
}

pub struct ThermalReadWrite(Box<dyn SerialPort>);

impl io::Read for ThermalReadWrite {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, io::Error> {
        self.0.read(buf)
    }
}

impl io::Write for ThermalReadWrite {
    fn write(&mut self, buf: &[u8]) -> Result<usize, io::Error> {
        self.0.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.0.flush()
    }
}

impl<'a> PortOpener<'a> for SerialPortOpener<'a> {
    type RW = ThermalReadWrite;

    fn open(&mut self) -> anyhow::Result<Self::RW> {
        let mut port_path: Option<String> = None;

        for port in serialport::available_ports()? {
            if let serialport::SerialPortType::UsbPort(port_info) = port.port_type {
                if port_info.vid == 0x303a && port_info.pid == 0x4001 {
                    port_path = Some(port.port_name);
                }
            }
        }

        match port_path {
            Some(port_path) => {
                let port = serialport::new(port_path, 921_600)
                    .timeout(Duration::from_secs(1))
                    .open();

                port.map(ThermalReadWrite)
                    .map_err(|e| anyhow!("Failed to open port: {e}"))
            }
            None => Err(anyhow!("Failed to find serial port")),
        }
    }
}
