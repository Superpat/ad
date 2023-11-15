//! https://groups.google.com/g/plan9port-dev/c/Zef5lM0HgnM
use ad::ninep::protocol::{Format9p, Rmessage, Tdata, Tmessage};
use std::{env, os::unix::net::UnixStream};

fn main() {
    let uname = env::var("USER").unwrap();
    let mntp = format!("/tmp/ns.{uname}.:0/acme");
    let mut socket = UnixStream::connect(mntp).unwrap();

    send(
        u16::MAX,
        Tdata::Version {
            msize: u16::MAX as u32,
            version: "9P2000".to_string(),
        },
        &mut socket,
    );

    send(
        0,
        Tdata::Attach {
            fid: 0,
            afid: 4294967295,
            uname,
            aname: "".to_string(),
        },
        &mut socket,
    );

    send(
        0,
        Tdata::Walk {
            fid: 0,
            new_fid: 1,
            wnames: vec![],
        },
        &mut socket,
    );

    send(0, Tdata::Stat { fid: 1 }, &mut socket);
    send(0, Tdata::Clunk { fid: 1 }, &mut socket);

    send(
        0,
        Tdata::Walk {
            fid: 0,
            new_fid: 1,
            wnames: vec![],
        },
        &mut socket,
    );

    send(0, Tdata::Open { fid: 1, mode: 0 }, &mut socket);
}

fn send(tag: u16, content: Tdata, socket: &mut UnixStream) {
    let t = Tmessage { tag, content };
    println!("t-message: {t:?}");
    t.write_to(socket).unwrap();

    let r = Rmessage::read_from(socket).unwrap();
    println!("r-message: {r:?}\n");
}
