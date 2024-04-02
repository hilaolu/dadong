use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpStream, UdpSocket};
use tokio::select;

use std::sync::Arc;

use lazy_static::lazy_static;

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

use tokio::net::tcp::OwnedReadHalf;
use tokio::net::tcp::OwnedWriteHalf;

async fn read_packet_from_tcp(
    tcp_stream: &mut OwnedReadHalf,
    buf: &mut [u8],
) -> std::io::Result<u16> {
    let size = tcp_stream.read_u16().await?;
    tcp_stream.read_exact(&mut buf[..size as usize]).await?;
    Ok(size)
}

async fn write_packet_to_tcp(
    tcp_stream: &mut OwnedWriteHalf,
    packet: &[u8],
) -> std::io::Result<()> {
    tcp_stream.write_u16(packet.len() as u16).await?;
    tcp_stream.write_all(packet).await?;
    Ok(())
}

async fn handle(tcp_stream: TcpStream, udp_stream: UdpSocket, addr: String) -> std::io::Result<()> {
    let (mut tcp_in, mut tcp_out) = tcp_stream.into_split();
    let udp_stream = Arc::new(udp_stream);
    let (udp_in, udp_out) = (udp_stream.clone(), udp_stream);

    let tcp2udp = async move {
        let mut buf = [0; 4096];
        let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(300));

        loop {
            select! {
                result=read_packet_from_tcp(&mut tcp_in,&mut buf)=>{
                   if let Ok(size)=result{
                       let _=udp_out.send_to(&buf[..size as usize], &addr).await;
                       interval.reset();
                   }else{
                       //tcp close here
                       break;
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
            if let Ok(size) = udp_in.recv(&mut buf).await {
                if size == 0 {
                    //udp close here
                    break;
                }

                let result = write_packet_to_tcp(&mut tcp_out, &buf[..size as usize]).await;

                if result.is_err() {
                    //tcp close here
                    break;
                }
            };
        }
    };

    tokio::spawn(udp2tcp);
    tokio::spawn(tcp2udp);

    Ok(())
}
