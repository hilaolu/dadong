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
    let client_addr = &TCP_ADDR;

    loop {
        let result = try_connect(client_addr).await;
        if result.is_err() {
            tokio::time::sleep(tokio::time::Duration::from_millis(1000)).await;
        }
    }
}

async fn try_connect(local_addr: &str) -> std::io::Result<()> {
    println!("Create connection, awaiting for response!");
    let mut stream = TcpStream::connect(local_addr).await?;

    // Accept an incoming connection
    let addr_length = stream.read_u8().await?;
    let mut addr = vec![0u8; addr_length as usize];
    stream.read_exact(&mut addr).await?;
    let addr = String::from_utf8(addr).unwrap_or_default();

    let udp_listener = UdpSocket::bind("0.0.0.0:0").await?;
    println!("Handshake success!");
    let _ = tokio::spawn(async move {
        let _ = handle(stream, udp_listener, addr).await;
    });
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
                   let _ = send_udp_from_buf(&mut udp_out,&mut buf,&addr).await;
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
        let mut buf = [0; 4096];
        loop {
            if let Ok(size) = udp_in.recv(&mut buf[2..]).await {
                if size == 0 {
                    //udp close here
                    break;
                }

                //write u16 size into the first 2 bytes of buf
                buf[0] = ((size >> 8) & 0xFF) as u8;
                buf[1] = (size & 0xFF) as u8;

                let result = tcp_out.write_all(&buf[..(size + 2) as usize]).await;

                if result.is_err() {
                    //tcp close here
                    break;
                }
            };
        }
    };

    tokio::spawn(async move {
        pin_mut!(udp2tcp);
        pin_mut!(tcp2udp);
        futures::future::select(udp2tcp, tcp2udp).await;
    });

    Ok(())
}
