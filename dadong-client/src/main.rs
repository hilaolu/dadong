use futures::{io, pin_mut};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, UdpSocket};
use tokio::select;

use lazy_static::lazy_static;

use dadong_lib::{read_tcp_to_buf, send_udp_from_buf, CircularBuffer};

lazy_static! {
    static ref TCP_ADDR: String = std::env::var("tcp_addr").expect("tcp_addr must be set.");
    static ref UDP_ADDR: String = std::env::var("udp_addr").expect("udp_addr must be set.");
    static ref REMOTE_ADDR: String =
        std::env::var("remote_addr").expect("remote_addr must be set.");
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
                let port=addr.port();
                let addr=addr.to_string();
                let mut pkt=vec![((size>>8) & 0xFF) as u8,(size & 0xFF) as u8];
                pkt.extend(&buf[..size]);

                if let Some(tx)=addr2handler.get(&addr){
                    let _=tx.send(pkt).await;
                }else{
                    //create a new tcp tunnel
                    let (stream, remote) = {
                        loop{
                            let (mut stream, remote)=tcp_listenser.accept().await.unwrap();

                            //Address Handshake
                            let target=&REMOTE_ADDR;
                            let len=target.len() as u8;
                            let _=stream.write_u16(port).await;
                            let _=stream.write_u8(len).await;
                            let _=stream.write_all(target.as_bytes()).await;
                            let _=stream.write_all(&pkt).await;
                            if let Ok(p)=stream.read_u16().await{
                                if p==port{
                                    break (stream, remote);
                                }
                            }
                        }
                    };

                    let (mut tcp_in,mut tcp_out)=stream.into_split();


                    println!("create a new tcp tunnel {}",remote);

                    //create a new handler in new tokio task
                    let (tx, mut rx) = tokio::sync::mpsc::channel(4);


                    addr2handler.insert(addr.clone(),tx);

                    let mut udp_out=udp_listener.clone();
                    let stale_tx=stale_tx.clone();

                    let addr_=addr.clone();
                    let udp2tcp=async move{
                        //set a timeout event
                        let mut interval=tokio::time::interval(tokio::time::Duration::from_secs(300));

                        loop{
                            select!{
                                Some(data)=rx.recv()=>{
                                    let result=tcp_out.write_all(&data).await;
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
                        let mut buf=CircularBuffer::new();
                        loop{
                            let result=read_tcp_to_buf(&mut tcp_in,&mut buf).await;
                            let _ = send_udp_from_buf(&mut udp_out,&mut buf,&addr).await;

                            match result{
                                Ok(0)=>{break;}
                                Err(_)=>{break;}
                                Ok(_)=>{}
                            }

                        }
                        //tcp close here
                    };

                    tokio::spawn(async move{
                        pin_mut!(udp2tcp);
                        pin_mut!(tcp2udp);
                        futures::future::select(udp2tcp,tcp2udp).await;
                    });

                }

            }
        }
    }
}
