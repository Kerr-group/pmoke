use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::time::Duration;

enum DhoTransport {
    Tcp(BufReader<TcpStream>),
    #[cfg(target_os = "windows")]
    Visa(gpib_rs::Instrument),
}

pub struct DHO5108 {
    transport: DhoTransport,
}

#[allow(dead_code)]
impl DHO5108 {
    pub fn open(ip: &str, port: u16, timeout: Option<Duration>) -> io::Result<Self> {
        let addr = format!("{}:{}", ip, port);
        let stream = TcpStream::connect(addr)?;
        stream.set_read_timeout(timeout)?;
        stream.set_write_timeout(timeout)?;
        stream.set_nodelay(true)?;
        let reader = BufReader::new(stream);
        Ok(DHO5108 {
            transport: DhoTransport::Tcp(reader),
        })
    }

    pub fn open_usbtmc(resource: &str, timeout: Option<Duration>) -> io::Result<Self> {
        #[cfg(target_os = "windows")]
        {
            let instrument = gpib_rs::Instrument::open_resource(resource, timeout)
                .map_err(|error| io::Error::other(error.to_string()))?;
            Ok(Self {
                transport: DhoTransport::Visa(instrument),
            })
        }

        #[cfg(not(target_os = "windows"))]
        {
            let _ = (resource, timeout);
            Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "USB-TMC currently requires NI-VISA on Windows",
            ))
        }
    }

    fn close(self) {}

    fn write_raw(&mut self, data: &[u8]) -> io::Result<()> {
        match &mut self.transport {
            DhoTransport::Tcp(reader) => {
                reader.get_mut().write_all(data)?;
                reader.get_mut().flush()
            }
            #[cfg(target_os = "windows")]
            DhoTransport::Visa(instrument) => instrument
                .write_raw(data)
                .map_err(|error| io::Error::other(error.to_string())),
        }
    }

    fn tcp_reader_mut(&mut self) -> &mut BufReader<TcpStream> {
        #[cfg(target_os = "windows")]
        match &mut self.transport {
            DhoTransport::Tcp(reader) => reader,
            DhoTransport::Visa(_) => unreachable!("VISA transport is handled separately"),
        }

        #[cfg(not(target_os = "windows"))]
        {
            let DhoTransport::Tcp(reader) = &mut self.transport;
            reader
        }
    }

    pub fn write_line(&mut self, cmd: &str) -> io::Result<()> {
        let s = format!("{cmd}\n");
        self.write_raw(s.as_bytes())
    }

    fn write_lines(&mut self, commands: &[String]) -> io::Result<()> {
        let capacity = commands.iter().map(|command| command.len() + 1).sum();
        let mut request = String::with_capacity(capacity);
        for command in commands {
            request.push_str(command);
            request.push('\n');
        }
        self.write_raw(request.as_bytes())
    }

    pub fn read_line(&mut self) -> io::Result<String> {
        match &mut self.transport {
            DhoTransport::Tcp(reader) => {
                let mut s = String::new();
                reader.read_line(&mut s)?;
                Ok(s.trim().to_string())
            }
            #[cfg(target_os = "windows")]
            DhoTransport::Visa(instrument) => instrument
                .read_string()
                .map_err(|error| io::Error::other(error.to_string())),
        }
    }

    pub fn query(&mut self, cmd: &str) -> io::Result<String> {
        self.write_line(cmd)?;
        self.read_line()
    }

    pub fn query_binary(&mut self, cmd: &str) -> io::Result<Vec<u8>> {
        #[cfg(target_os = "windows")]
        if let DhoTransport::Visa(instrument) = &mut self.transport {
            let mut data = Vec::new();
            instrument
                .query_ieee_block(cmd, &mut data)
                .map_err(|error| io::Error::other(error.to_string()))?;
            return Ok(data);
        }

        self.write_line(cmd)?;

        let reader = self.tcp_reader_mut();

        // SCPI binary block structure:
        // 1  1         n             length          1
        // # <n> <length_header> <binary_data> [<terminator>]

        // SCPI binary block starts with '#'
        let mut one = [0u8; 1];
        reader.read_exact(&mut one)?;
        if one[0] != b'#' {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Expected '#' at the start of binary block",
            ));
        }

        reader.read_exact(&mut one)?;
        let n = (one[0] - b'0') as usize;

        let mut len_buf = vec![0u8; n];
        reader.read_exact(&mut len_buf)?;
        let len_str = String::from_utf8(len_buf)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        let length: usize = len_str
            .parse()
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;

        let mut data = vec![0u8; length];
        reader.read_exact(&mut data)?;

        let _ = reader.read(&mut one);

        Ok(data)
    }

    pub fn identify(&mut self) -> io::Result<String> {
        self.query("*IDN?")
    }

    pub fn fetch(&mut self, ch: u8, memory_depth: usize) -> io::Result<Vec<f64>> {
        // Send sequential setup commands in one write and synchronize once at the end.
        self.write_lines(&[
            format!("WAV:SOUR CHAN{ch}"),
            "WAV:MODE RAW".to_string(),
            "WAV:FORM WORD".to_string(),
            "WAV:STAR 1".to_string(),
            format!("WAV:POIN {memory_depth}"),
            format!("WAV:STOP {memory_depth}"),
            "*OPC?".to_string(),
        ])?;
        let _ = self.read_line()?; // "1"

        // PREamble returns all scaling fields in one query.
        let preamble = self.query("WAV:PRE?")?;
        let fields: Vec<&str> = preamble.split(',').collect();
        if fields.len() != 10 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("expected 10 waveform preamble fields, got {}", fields.len()),
            ));
        }
        let parse_field = |index: usize, name: &str| -> io::Result<f64> {
            fields[index].parse().map_err(|error| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("invalid waveform preamble {name}: {error}"),
                )
            })
        };
        let y_inc = parse_field(7, "yincrement")?;
        let y_ori = parse_field(8, "yorigin")?;
        let y_ref = parse_field(9, "yreference")?;

        // read actual data
        let raw_bytes = self.query_binary("WAV:DATA?")?;

        if raw_bytes.len() % 2 != 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "odd length binary",
            ));
        }

        let mut result = Vec::with_capacity(raw_bytes.len() / 2);
        for chunk in raw_bytes.chunks_exact(2) {
            let v = u16::from_le_bytes([chunk[0], chunk[1]]) as f64;
            let y = (v - y_ori - y_ref) * y_inc;
            result.push(y);
        }

        Ok(result)
    }

    pub fn set_single(&mut self) -> io::Result<()> {
        self.write_line("TRIG:SWE SING")?;
        Ok(())
    }
}
