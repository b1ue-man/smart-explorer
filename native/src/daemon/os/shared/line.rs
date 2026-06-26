use std::io::{self, Read};
use std::net::TcpStream;

pub(super) const MAX_IPC_LINE: usize = 256 * 1024;

pub(super) fn read_line_limited_from_stream(
    stream: &mut TcpStream,
    line: &mut String,
    max: usize,
) -> io::Result<usize> {
    line.clear();
    let mut bytes = Vec::new();
    let mut byte = [0u8; 1];
    loop {
        match stream.read(&mut byte) {
            Ok(0) => break,
            Ok(n) => {
                if bytes.len().saturating_add(n) > max {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "ipc line too large",
                    ));
                }
                bytes.extend_from_slice(&byte[..n]);
                if byte[0] == b'\n' {
                    break;
                }
            }
            Err(e) => return Err(e),
        }
    }
    let n = bytes.len();
    *line = String::from_utf8(bytes)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "ipc line invalid utf8"))?;
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::{read_line_limited_from_stream, MAX_IPC_LINE};
    use std::io::Write;
    use std::net::{TcpListener, TcpStream};

    fn connected_pair() -> (TcpStream, TcpStream) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let client = TcpStream::connect(addr).unwrap();
        let (server, _) = listener.accept().unwrap();
        (client, server)
    }

    #[test]
    fn stream_reader_rejects_oversized_line() {
        let (mut client, mut server) = connected_pair();
        let payload = vec![b'a'; MAX_IPC_LINE + 1];
        client.write_all(&payload).unwrap();
        client.shutdown(std::net::Shutdown::Write).unwrap();

        let mut line = String::new();
        assert!(read_line_limited_from_stream(&mut server, &mut line, MAX_IPC_LINE).is_err());
    }
}
