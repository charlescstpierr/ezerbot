//! Chrome ouvre plusieurs connexions keep-alive d'un coup. Chacune doit être lue,
//! même tant que les autres restent ouvertes (vendor/tiny_http/EZERBOT.md).
use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::{Duration, Instant};

#[test]
fn a_burst_of_keep_alive_connections_is_read_while_the_others_stay_open() {
    for round in 0..20 {
        let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
        let port = server.server_addr().to_ip().unwrap().port();
        let count = 12;
        let request =
            format!("GET / HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: keep-alive\r\n\r\n");
        // Toutes ouvertes et écrites avant toute réponse ; aucune n'est fermée.
        let mut clients: Vec<TcpStream> = (0..count)
            .map(|_| TcpStream::connect(("127.0.0.1", port)).unwrap())
            .collect();
        for client in &mut clients {
            client.write_all(request.as_bytes()).unwrap();
        }
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut answered = 0;
        while answered < count {
            let left = deadline.saturating_duration_since(Instant::now());
            match server.recv_timeout(left).unwrap() {
                Some(request) => {
                    request.respond(tiny_http::Response::from_string("ok")).unwrap();
                    answered += 1;
                }
                None => panic!("round {round}: {answered}/{count} connexions lues, les autres attendent un fil"),
            }
        }
        for client in &mut clients {
            client
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut first = [0u8; 1];
            client.read_exact(&mut first).unwrap();
        }
    }
}
