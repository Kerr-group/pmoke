use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::time::Duration;

pub struct DHO5108 {
    reader: BufReader<TcpStream>,
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
        Ok(DHO5108 { reader })
    }

    fn inner_mut(&mut self) -> &mut TcpStream {
        self.reader.get_mut()
    }

    fn close(self) {}

    pub fn write_line(&mut self, cmd: &str) -> io::Result<()> {
        let s = format!("{cmd}\n");
        self.inner_mut().write_all(s.as_bytes())?;
        self.inner_mut().flush()?;
        Ok(())
    }

    fn write_lines(&mut self, commands: &[String]) -> io::Result<()> {
        let capacity = commands.iter().map(|command| command.len() + 1).sum();
        let mut request = String::with_capacity(capacity);
        for command in commands {
            request.push_str(command);
            request.push('\n');
        }
        self.inner_mut().write_all(request.as_bytes())?;
        self.inner_mut().flush()
    }

    pub fn read_line(&mut self) -> io::Result<String> {
        let mut s = String::new();
        self.reader.read_line(&mut s)?;
        Ok(s.trim().to_string())
    }

    pub fn query(&mut self, cmd: &str) -> io::Result<String> {
        self.write_line(cmd)?;
        self.read_line()
    }

    pub fn query_binary(&mut self, cmd: &str) -> io::Result<Vec<u8>> {
        self.write_line(cmd)?;

        // SCPI binary block structure:
        // 1  1         n             length          1
        // # <n> <length_header> <binary_data> [<terminator>]

        // SCPI binary block starts with '#'
        let mut one = [0u8; 1];
        self.reader.read_exact(&mut one)?;
        if one[0] != b'#' {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Expected '#' at the start of binary block",
            ));
        }

        self.reader.read_exact(&mut one)?;
        let n = (one[0] - b'0') as usize;

        let mut len_buf = vec![0u8; n];
        self.reader.read_exact(&mut len_buf)?;
        let len_str = String::from_utf8(len_buf)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        let length: usize = len_str
            .parse()
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;

        let mut data = vec![0u8; length];
        self.reader.read_exact(&mut data)?;

        let _ = self.reader.read(&mut one);

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
