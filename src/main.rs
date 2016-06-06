extern crate mio;
extern crate bytes;

use bytes::Take;
use mio::{EventLoop, EventSet, Handler, PollOpt, Token, TryRead, TryWrite};
use mio::tcp::{Shutdown, TcpListener, TcpStream};
use mio::util::Slab;
use std::io::BufWriter;
use std::io::Cursor;
use std::io::Write;

const SERVER: Token = Token(0);

#[derive(Debug)]
enum StatusCode {
    Ok,
    Error,
}

impl StatusCode {
    fn write(&self, buf: &mut BufWriter<&mut Vec<u8>>) -> Result<usize, std::io::Error> {
        match *self {
            StatusCode::Ok => buf.write(b"HTTP/1.1 200 OK\r\n"),
            StatusCode::Error => buf.write(b"HTTP/1.1 500 Internal Server Error\r\n"),
        }
    }
}

#[derive(Debug)]
struct Response {
    code: StatusCode,
    body: Vec<u8>,
}

impl Response {
    fn new() -> Response {
        Response {
            code: StatusCode::Ok,
            body: vec![],
        }
    }

    fn write(&self, buf: &mut BufWriter<&mut Vec<u8>>) -> Result<usize, std::io::Error> {
        try!(self.code.write(buf));
        try!(write!(buf, "Content-Lenght: {}\r\n", self.body.len()));
        try!(buf.write(b"Content-type: text/plain; charset=UTF-8\r\n"));
        try!(buf.write(b"\r\n"));
        try!(buf.write(&self.body));

        Ok(0)
    }
}

#[derive(Debug)]
enum State {
    Reading,
    Handling,
    Writing,
}

#[derive(Debug)]
struct Request {
    stream: TcpStream,
    token: Token,
    state: State,
    req: Vec<u8>,
    res: Vec<u8>,
}

impl Request {
    fn new(stream: TcpStream, token: Token) -> Request {
        Request {
            stream: stream,
            token: token,
            state: State::Reading,
            req: vec![],
            res: vec![],
        }
    }

    fn ready(&mut self, event_loop: &mut EventLoop<HttpServer>, events: EventSet) {
        match self.state {
            State::Reading => {
                assert!(events.is_readable(), "not readable; {:?}", events);
                self.read(event_loop);
            }
            State::Handling => {
                self.handle(event_loop);
            }
            State::Writing => {
                assert!(events.is_writable(), "not writable; {:?}", events);
                self.write(event_loop);
            }
        }
    }

    fn read(&mut self, event_loop: &mut EventLoop<HttpServer>) {
        match self.stream.try_read_buf(&mut self.req) {
            Ok(Some(0)) => println!("read 0 bytes!!"),
            Ok(Some(n)) => {
                println!("read {} bytes", n);
                if self.req.ends_with(&"\r\n\r\n".to_string().as_bytes()) {
                    println!("found end!");
                    self.state = State::Handling;
                }
                self.reregister(event_loop);
            }
            Ok(None) => {
                println!("read nothing");
                self.reregister(event_loop);
            }
            Err(e) => {
                panic!("read error! {:?}", e);
            }
        }
        // println!("=====");
        // match String::from_utf8(self.buf.clone()) {
        //     Ok(s) => println!("{:?}", s),
        //     Err(e) => println!("err! {}", e),
        // };
        // println!("=====");
    }

    fn handle(&mut self, event_loop: &mut EventLoop<HttpServer>) {
        let mut response = Response::new();
        {
            let mut buf = BufWriter::new(&mut response.body);
            buf.write("Hello World!".to_string().as_bytes()).unwrap();
        }

        {
            let mut buf = BufWriter::new(&mut self.res);
            response.write(&mut buf).unwrap();
        }

        self.state = State::Writing;
        self.reregister(event_loop);
    }

    fn write(&mut self, event_loop: &mut EventLoop<HttpServer>) {
        let len = self.res.len();

        println!("bytes {}", &len);
        let cur = Cursor::new(self.res.as_slice());
        let mut tak = Take::new(cur, len);
        match self.stream.try_write_buf(&mut tak) {
            Ok(Some(n)) => {
                println!("wrote {} bytes", n);
            }
            Ok(None) => {
                println!("wrote nothing");
                self.reregister(event_loop);
            }
            Err(e) => {
                panic!("write error! {:?}", e);
            }
        }
        println!("shutting down!");
        self.stream.shutdown(Shutdown::Both).unwrap();
    }

    fn reregister(&self, event_loop: &mut EventLoop<HttpServer>) {
        println!("registering {:?}", &self.state);
        let event_set = match self.state {
            State::Reading => EventSet::readable(),
            State::Handling => EventSet::all(),
            State::Writing => EventSet::writable(),
        };

        event_loop.reregister(&self.stream, self.token, event_set, PollOpt::oneshot()).unwrap()
    }
}

#[derive(Debug)]
struct HttpServer {
    listener: TcpListener,
    requests: Slab<Request>, // request_handler:
}

impl HttpServer {
    fn new(listener: TcpListener) -> HttpServer {
        let slab = Slab::new_starting_at(Token(1), 1024);

        HttpServer {
            listener: listener,
            requests: slab,
        }
    }
}

impl Handler for HttpServer {
    type Timeout = ();
    type Message = ();

    fn ready(&mut self, event_loop: &mut EventLoop<Self>, token: Token, events: EventSet) {
        match token {
            SERVER => {
                assert!(events.is_readable());
                match self.listener.accept() {
                    Ok(Some((stream, addr))) => {
                        println!("accepted a connection on {:?}", addr);
                        let token = self.requests
                            .insert_with(|token| Request::new(stream, token))
                            .unwrap();

                        event_loop.register(&self.requests[token].stream,
                                      token,
                                      EventSet::readable(),
                                      PollOpt::edge() | PollOpt::oneshot())
                            .unwrap()
                    }
                    Ok(None) => {
                        println!("false alarm!");
                    }
                    Err(e) => {
                        println!("error! {:?}", e);
                    }
                }
            }
            _ => {
                self.requests[token].ready(event_loop, events);
            }
        }
    }
}

fn main() {
    println!("Hey, listen! http://127.0.0.1:13265");

    let addr = "127.0.0.1:13265".parse().unwrap();
    let listener = TcpListener::bind(&addr).unwrap();
    let mut event_loop = EventLoop::new().unwrap();

    event_loop.register(&listener, SERVER, EventSet::readable(), PollOpt::edge())
        .unwrap();

    let mut http_server = HttpServer::new(listener);
    event_loop.run(&mut http_server).unwrap();
}
