//! The actual App event producer over the native server, with persisted runs and no providers.
use ecdev_core::{Engine, json, json::Value};
use ecdev_net::server::{Config, Server, ShutdownHandle};
use ecdev_server::web::App;
use std::{
    io::{BufRead, BufReader, Read, Write},
    net::{SocketAddr, TcpStream},
    path::PathBuf,
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

struct Running {
    root: PathBuf,
    addr: SocketAddr,
    stop: ShutdownHandle,
    join: Option<thread::JoinHandle<()>>,
}

impl Running {
    fn start(engine: Engine, root: PathBuf) -> Self {
        let server = Server::bind("127.0.0.1:0", Config::default()).unwrap();
        let addr = server.local_addr().unwrap();
        let stop = server.shutdown_handle().unwrap();
        let join = thread::spawn(move || server.serve(App::new(engine)).unwrap());
        Self {
            root,
            addr,
            stop,
            join: Some(join),
        }
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        self.stop.shutdown();
        if let Some(join) = self.join.take() {
            join.join().unwrap();
        }
        std::fs::remove_dir_all(&self.root).unwrap();
    }
}

fn connect(addr: SocketAddr, target: &str, last_event_id: Option<u64>) -> BufReader<TcpStream> {
    let mut socket = TcpStream::connect(addr).unwrap();
    socket
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    write!(socket, "GET {target} HTTP/1.1\r\nHost: {addr}\r\n").unwrap();
    if let Some(id) = last_event_id {
        write!(socket, "Last-Event-ID: {id}\r\n").unwrap();
    }
    socket.write_all(b"\r\n").unwrap();
    let mut reader = BufReader::new(socket);
    let mut line = String::new();
    reader.read_line(&mut line).unwrap();
    assert_eq!(line, "HTTP/1.1 200 OK\r\n");
    let mut headers = String::new();
    loop {
        line.clear();
        reader.read_line(&mut line).unwrap();
        assert!(!line.is_empty(), "response ended before its head");
        if line == "\r\n" {
            break;
        }
        headers.push_str(&line);
    }
    assert!(headers.contains("Content-Type: text/event-stream\r\n"));
    assert!(headers.contains("Transfer-Encoding: chunked\r\n"));
    reader
}

fn next_event(reader: &mut BufReader<TcpStream>) -> Value {
    let mut line = String::new();
    reader.read_line(&mut line).unwrap();
    assert!(line.ends_with("\r\n"));
    let size = usize::from_str_radix(line.trim_end(), 16).unwrap();
    assert!((1..=1024 * 1024).contains(&size));
    let mut data = vec![0; size];
    reader.read_exact(&mut data).unwrap();
    let mut end = [0; 2];
    reader.read_exact(&mut end).unwrap();
    assert_eq!(&end, b"\r\n");
    let text = String::from_utf8(data).unwrap();
    let lines: Vec<_> = text.lines().collect();
    assert_eq!(lines.len(), 4, "{text:?}");
    let id = lines[0]
        .strip_prefix("id: ")
        .unwrap()
        .parse::<u64>()
        .unwrap();
    assert_eq!(lines[1], "event: run");
    assert_eq!(lines[3], "");
    let data: Value = json::from_str(lines[2].strip_prefix("data: ").unwrap()).unwrap();
    json!({"id":id,"data":data})
}

fn store_run(engine: &Engine, asin: &str) -> Value {
    let run = engine
        .call(
            "ecdev.product.analyze",
            json!({"market":"AMAZON_JP","asin":asin}),
        )
        .unwrap();
    assert_eq!(run["mode"], "PLAN_ONLY");
    assert_eq!(run["network_calls"], 0);
    assert_eq!(run["cost_minor"], 0);
    assert_eq!(run["observations"], json!([]));
    run
}

#[test]
fn persisted_events_replay_reconnect_and_stop_through_the_real_app() {
    let root = std::env::temp_dir().join(format!(
        "ecdev-event-stream-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let engine = Engine::open(&root).unwrap();
    for asin in ["B000000001", "B000000002", "B000000003"] {
        store_run(&engine, asin);
    }
    let recorded = engine.events(0).unwrap();
    let recorded = recorded.as_array().unwrap();
    assert_eq!(recorded.len(), 3);
    let running = Running::start(engine.clone(), root);

    let mut first = connect(running.addr, "/events?after=0", None);
    for event in recorded {
        assert_eq!(&next_event(&mut first), event);
    }
    drop(first);

    // Last-Event-ID takes precedence over the query cursor and excludes the acknowledged row.
    let mut resumed = connect(
        running.addr,
        "/events?after=999999",
        recorded[0]["id"].as_u64(),
    );
    assert_eq!(next_event(&mut resumed), recorded[1]);
    assert_eq!(next_event(&mut resumed), recorded[2]);
    let live = store_run(&engine, "B000000004");
    let delivered = next_event(&mut resumed);
    let expected = engine.events(recorded[2]["id"].as_u64().unwrap()).unwrap();
    assert_eq!(delivered, expected[0]);
    assert_eq!(delivered["data"]["run_id"], live["run_id"]);

    running.stop.shutdown();
    let retained_after_shutdown = store_run(&engine, "B000000005");
    let mut tail = Vec::new();
    resumed.read_to_end(&mut tail).unwrap();
    // An idle producer can finish normally; a send racing shutdown can end without a terminator.
    assert!(tail.is_empty() || tail == b"0\r\n\r\n", "{tail:?}");
    let all = engine.events(0).unwrap();
    let all = all.as_array().unwrap();
    assert_eq!(all.len(), 5);
    assert_eq!(&all[..3], recorded);
    assert_eq!(all[4]["data"]["run_id"], retained_after_shutdown["run_id"]);
    drop(resumed);
    drop(engine);
}
