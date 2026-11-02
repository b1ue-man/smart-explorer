//! Scripted FTP wire fixture shared by transfer/provider acceptance cases.
use super::*;

struct OpenGuard(Arc<AtomicUsize>);

impl Drop for OpenGuard {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

fn reply(stream: &mut TcpStream, line: &str) -> io::Result<()> {
    stream.write_all(line.as_bytes())?;
    stream.write_all(b"\r\n")?;
    stream.flush()
}

fn child_name<'a>(dir: &str, path: &'a str) -> Option<&'a str> {
    let (parent, name) = path.rsplit_once('/')?;
    (parent == dir && !name.is_empty()).then_some(name)
}

fn listing(store: &Store, dir: &str) -> String {
    let dir = dir.trim_end_matches('/');
    let mut lines = String::new();
    for path in &store.dirs {
        if let Some(name) = child_name(dir, path) {
            lines.push_str(&format!("drwxr-xr-x 1 0 0 0 Jan 1 2020 {name}\r\n"));
        }
    }
    for (path, content) in &store.files {
        if let Some(name) = child_name(dir, path) {
            let size = content.len();
            lines.push_str(&format!("-rw-r--r-- 1 0 0 {size} Jan 1 2020 {name}\r\n"));
        }
    }
    lines
}

/// 150, one passive data connection for `work`, then 226.
fn transfer(
    control: &mut TcpStream,
    passive: Option<TcpListener>,
    work: impl FnOnce(&mut TcpStream) -> io::Result<()>,
) -> io::Result<()> {
    let Some(listener) = passive else {
        return reply(control, "425 Use PASV first");
    };
    reply(control, "150 opening data connection")?;
    listener.set_nonblocking(true)?;
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut data = loop {
        match listener.accept() {
            Ok((data, _)) => break data,
            Err(error)
                if error.kind() == io::ErrorKind::WouldBlock && Instant::now() < deadline =>
            {
                thread::sleep(Duration::from_millis(2));
            }
            Err(error) => return Err(error),
        }
    };
    data.set_nonblocking(false)?;
    data.set_read_timeout(Some(Duration::from_secs(5)))?;
    work(&mut data)?;
    drop(data);
    reply(control, "226 transfer complete")
}

pub(super) fn serve(
    mut stream: TcpStream,
    rules: &Rules,
    store: &Mutex<Store>,
    open: &Arc<AtomicUsize>,
) {
    let _ = stream.set_nonblocking(false);
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    let over = open.fetch_add(1, Ordering::SeqCst) + 1 > rules.max_connections;
    let _guard = OpenGuard(open.clone());
    if over && rules.refuse == Refuse::Greeting {
        let _ = reply(
            &mut stream,
            "421 There are too many connections from your internet address.",
        );
        return;
    }
    if reply(&mut stream, "220 script ready").is_err() {
        return;
    }
    let Ok(reader) = stream.try_clone() else {
        return;
    };
    let mut reader = BufReader::new(reader);
    let mut passive: Option<TcpListener> = None;
    let mut rest = 0usize;
    let mut current = "/".to_string();
    loop {
        let mut line = String::new();
        match reader.read_line(&mut line) {
            Ok(0) | Err(_) => return,
            Ok(_) => {}
        }
        let line = line.trim_end_matches(['\r', '\n']).to_string();
        let (command, argument) = match line.split_once(' ') {
            Some((command, argument)) => (command.to_string(), argument.to_string()),
            None => (line.clone(), String::new()),
        };
        lock(store).commands.push(line.clone());
        let answered = match command.as_str() {
            "USER" => reply(&mut stream, "331 password required"),
            "PASS" if rules.wrong_password => reply(&mut stream, "530 Login incorrect."),
            "PASS" if over => reply(
                &mut stream,
                "530 Sorry, the maximum number of clients (1) for this user are already connected.",
            ),
            "PASS" => reply(&mut stream, "230 logged in"),
            "TYPE" => reply(&mut stream, "200 binary"),
            "PWD" => reply(
                &mut stream,
                &format!("257 \"{current}\" is current directory"),
            ),
            "CWD" if argument == "/" || lock(store).dirs.contains(&argument) => {
                current = argument.clone();
                reply(&mut stream, "250 directory changed")
            }
            "CWD" => reply(&mut stream, "550 No such directory"),
            "SIZE" => match lock(store).files.get(&argument) {
                Some(bytes) => reply(&mut stream, &format!("213 {}", bytes.len())),
                None => reply(&mut stream, "550 No readable regular file"),
            },
            "NOOP" => reply(&mut stream, "200 alive"),
            "QUIT" => {
                let _ = reply(&mut stream, "221 bye");
                return;
            }
            "MKD" => {
                let created = {
                    let mut store = lock(store);
                    let taken =
                        store.dirs.contains(&argument) || store.files.contains_key(&argument);
                    if !taken {
                        store.dirs.insert(argument.clone());
                    }
                    !taken
                };
                if created {
                    reply(&mut stream, &format!("257 \"{argument}\" created"))
                } else {
                    reply(&mut stream, "550 File exists")
                }
            }
            "PASV" => match TcpListener::bind("127.0.0.1:0") {
                Ok(listener) => {
                    let port = listener.local_addr().map_or(0, |address| address.port());
                    passive = Some(listener);
                    let (high, low) = (port / 256, port % 256);
                    reply(
                        &mut stream,
                        &format!("227 Entering Passive Mode (127,0,0,1,{high},{low})"),
                    )
                }
                Err(error) => Err(error),
            },
            "REST" if !rules.rest => reply(&mut stream, "502 Command not implemented"),
            "REST" => {
                rest = argument.parse().unwrap_or(0);
                reply(&mut stream, &format!("350 Restarting at {rest}"))
            }
            "LIST" => transfer(&mut stream, passive.take(), |data| {
                let folder = if argument == "-a" {
                    &current
                } else {
                    &argument
                };
                let lines = listing(&lock(store), folder);
                data.write_all(lines.as_bytes())
            }),
            "RETR" => {
                let offset = std::mem::take(&mut rest);
                let content = lock(store).files.get(&argument).cloned();
                match content {
                    Some(content) => transfer(&mut stream, passive.take(), |data| {
                        data.write_all(&content[offset.min(content.len())..])
                    }),
                    None => reply(&mut stream, "550 No such file"),
                }
            }
            "STOR" => transfer(&mut stream, passive.take(), |data| {
                let mut body = Vec::new();
                data.read_to_end(&mut body)?;
                lock(store).files.insert(argument.clone(), body);
                Ok(())
            }),
            _ => reply(&mut stream, "500 unsupported"),
        };
        if answered.is_err() {
            return;
        }
    }
}
