use futures::io;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, UdpSocket};
use tokio::select;

use tokio::net::tcp::OwnedReadHalf;
use tokio::net::tcp::OwnedWriteHalf;

use lazy_static::lazy_static;

lazy_static! {
    static ref TCP_ADDR: String = std::env::var("tcp_addr").expect("tcp_addr must be set.");
    static ref UDP_ADDR: String = std::env::var("udp_addr").expect("udp_addr must be set.");
    static ref REMOTE_ADDR: String =
        std::env::var("remote_addr").expect("remote_addr must be set.");
}

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
    tcp_stream.write_all(&packet).await?;
    Ok(())
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> io::Result<()> {
    let tcp_listenser = TcpListener::bind(TCP_ADDR.to_string()).await.unwrap();
    let udp_listener = Arc::new(UdpSocket::bind(UDP_ADDR.to_string()).await?);
    let mut addr2handler: HashMap<String, tokio::sync::mpsc::Sender<Vec<u8>>> = HashMap::new();
    let (stale_tx, mut stale_rx) = tokio::sync::mpsc::channel(4);

    let udp_in = udp_listener.clone();

    // Spawn a task to handle incoming packets
    let mut buf = [0; 4096];
    loop {
        select! {
            //remove stale mapping
            Some(addr)=stale_rx.recv()=>{
                addr2handler.remove(&addr);
            }

            Ok((size, addr))= udp_in.recv_from(&mut buf) => {
                let addr=addr.to_string();
                let data=Vec::from(&buf[..size]);

                if let Some(tx)=addr2handler.get(&addr){
                    let _=tx.send(data).await;
                }else{
                    //create a new tcp tunnel
                    let (stream, remote) = tcp_listenser.accept().await.unwrap();
                    println!("create a new tcp tunnel {}",remote);

                    //create a new handler in new tokio task
                    let (tx, mut rx) = tokio::sync::mpsc::channel(4);
                    let _=tx.send(data).await;

                    let (mut tcp_in,mut tcp_out)=stream.into_split();

                    //Address Handshake
                    let target=&REMOTE_ADDR;
                    let _=tcp_out.write_u8(target.len() as u8).await;
                    let _=tcp_out.write_all(target.as_bytes()).await;


                    addr2handler.insert(addr.clone(),tx);

                    let udp_out=udp_listener.clone();
                    let stale_tx=stale_tx.clone();

                    let addr_=addr.clone();
                    let udp2tcp=async move{
                        //set a timeout event
                        let mut interval=tokio::time::interval(tokio::time::Duration::from_secs(300));

                        loop{
                            select!{
                                Some(data)=rx.recv()=>{
                                    let result=write_packet_to_tcp(&mut tcp_out,&data).await;
                                    if result.is_ok(){
                                        //reset timeout
                                        interval.reset();
                                    }else{
                                        break;
                                    }
                                }
                                //timeout event
                                _=interval.tick()=>{
                                    break;
                                }
                            }
                        }
                        let _=stale_tx.send(addr_).await;
                    };

                    let tcp2udp=async move {
                        let mut buf=[0;4096];
                        //assume 4096 is much more larger than a single udp packet
                        loop{
                            let result=read_packet_from_tcp(&mut tcp_in,&mut buf).await;

                            if let Ok(size)=result{
                                if size==0{
                                    break;
                                }
                                let _=udp_out.send_to(&buf[..size as usize], &addr).await;
                            }else{
                                break;
                            }
                        }
                        //tcp close here
                    };

                    tokio::spawn(udp2tcp);
                    tokio::spawn(tcp2udp);

                }

            }
        }
    }
}
