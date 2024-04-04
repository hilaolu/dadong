use futures::pin_mut;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpStream, UdpSocket};
use tokio::select;

use std::sync::Arc;

use lazy_static::lazy_static;

use dadong_lib::{read_tcp_to_buf, send_udp_from_buf, CircularBuffer};

lazy_static! {
    static ref TCP_ADDR: String = std::env::var("tcp_addr").expect("tcp_addr must be set.");
}

#[tokio::main(flavor = "current_thread")]

async fn main() -> std::io::Result<()> {
    loop {
        let _ = server().await;
    }
}
async fn server() -> std::io::Result<()> {
    let client_addr = &TCP_ADDR;

    let mut control_stream = TcpStream::connect(client_addr.to_string()).await?;
    control_stream.write_u8(255).await?;
    match control_stream.read_u8().await {
        Ok(255) => {
            println!("Control channel established: {:?}", client_addr.to_string());
        }
        _ => {
            return Ok(());
        }
    }

    let (mut control_rx, mut control_tx) = control_stream.into_split();

    let dog = async move {
        loop {
            tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
            match control_tx.write_u8(0).await {
                Err(_) => {
                    println!("Disconnected");
                    break;
                }
                _ => {}
            }
        }
    };

    let manager = async move {
        loop {
            if let Ok(_) = control_rx.read_u8().await {
                let _ = tokio::spawn(async move {
                    let _ = try_connect(client_addr).await;
                });
            } else {
                break;
            };
        }
    };

    pin_mut!(dog);
    pin_mut!(manager);
    futures::future::select(dog, manager).await;

    Ok(())
}

async fn try_connect(local_addr: &str) -> std::io::Result<()> {
    println!("Create connection, awaiting for response!");
    let mut stream = TcpStream::connect(local_addr).await?;
    let addr_length = stream.read_u8().await?;
    let mut addr = vec![0u8; addr_length as usize];
    stream.read_exact(&mut addr).await?;
    let addr = String::from_utf8(addr).unwrap_or_default();
    println!("Handshake success!");

    // Accept an incoming connection
    let udp_listener = UdpSocket::bind("0.0.0.0:0").await?;
    let _ = handle(stream, udp_listener, addr).await;
    Ok(())
}

async fn handle(tcp_stream: TcpStream, udp_stream: UdpSocket, addr: String) -> std::io::Result<()> {
    let (mut tcp_in, mut tcp_out) = tcp_stream.into_split();
    let udp_stream = Arc::new(udp_stream);
    let (udp_in, mut udp_out) = (udp_stream.clone(), udp_stream);

    let tcp2udp = async move {
        let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(300));
        let mut buf = CircularBuffer::new();

        loop {
            select! {
                result=read_tcp_to_buf(&mut tcp_in,&mut buf)=>{
                    let _=send_udp_from_buf(&mut udp_out,&mut buf,&addr).await;
                    match result{
                        Ok(0)=>{break;}
                        Err(_)=>{break;}
                        Ok(_)=>{interval.reset();}
                    }
                }
                _=interval.tick()=>{
                    //timeout, shutdown tcp
                    break;
                }
            }
        }
    };

    let udp2tcp = async move {
        const MTU: usize = 1500;
        const BUFFER_SIZE: usize = 8192;
        const PKT_HEAD_LENGTH: usize = 2;
        let mut buf = [0; BUFFER_SIZE];
        loop {
            let _ = udp_in.readable().await;
            let mut head = 0;
            loop {
                if head + PKT_HEAD_LENGTH + MTU > BUFFER_SIZE {
                    break;
                }
                match udp_in.try_recv(&mut buf[head + PKT_HEAD_LENGTH..]) {
                    Ok(0) => {
                        break;
                    }
                    Ok(size) => {
                        //write u16 size into the first 2 bytes of buf
                        buf[head + 0] = ((size >> 8) & 0xFF) as u8;
                        buf[head + 1] = (size & 0xFF) as u8;

                        head += size + PKT_HEAD_LENGTH;
                    }
                    Err(_) => {
                        break;
                    }
                }
            }
            let result = tcp_out.write_all(&buf[..head as usize]).await;
            if result.is_err() {
                break;
            }
        }
    };

    tokio::spawn(async move {
        pin_mut!(udp2tcp);
        pin_mut!(tcp2udp);
        futures::future::select(udp2tcp, tcp2udp).await;
    });

    Ok(())
}
