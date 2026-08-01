use std::io::{self, Read, Write};

use crate::error::{Error, Result};

pub(crate) fn write_line(io: &mut impl Write, command: &str) -> Result<()> {
    io.write_all(command.as_bytes())?;
    io.write_all(b"\n")?;
    io.flush()?;
    Ok(())
}

pub(crate) fn read_response_bytes(io: &mut impl Read) -> Result<Vec<u8>> {
    let mut response = Vec::new();
    let mut byte = [0_u8; 1];
    loop {
        match io.read(&mut byte) {
            Ok(0) => break,
            Ok(_) => {
                response.push(byte[0]);
                if byte[0] == b'\n' {
                    break;
                }
            }
            Err(err)
                if err.kind() == io::ErrorKind::TimedOut
                    || err.kind() == io::ErrorKind::WouldBlock =>
            {
                if response.is_empty() {
                    return Err(Error::Io(err));
                }
                break;
            }
            Err(err) => return Err(Error::Io(err)),
        }
    }
    Ok(response)
}
