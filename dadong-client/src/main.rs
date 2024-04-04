use futures::{io, pin_mut};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream, UdpSocket};
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
    loop {
        let _ = server().await;
    }
}
async fn server() -> io::Result<()> {
    let tcp_listenser = TcpListener::bind(TCP_ADDR.to_string()).await.unwrap();
    let udp_listener = Arc::new(UdpSocket::bind(UDP_ADDR.to_string()).await?);
    let mut addr2handler: HashMap<String, tokio::sync::mpsc::Sender<Vec<u8>>> = HashMap::new();
    let (stale_tx, mut stale_rx) = tokio::sync::mpsc::channel(4);

    let udp_in = udp_listener.clone();

    let (mut control_stream, server_addr) = tcp_listenser.accept().await?;
    control_stream.write_u8(255).await?;
    match control_stream.read_u8().await {
        Ok(255) => {
            println!("Control channel established: {:?}", server_addr.to_string());
        }
        _ => {
            return Ok(());
        }
    }
    let (manager_tx, mut manager_rx) = tokio::sync::mpsc::channel(4);

    let manager = async move {
        let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(30));
        interval.reset();
        let mut waiting_worker: Vec<tokio::sync::mpsc::Sender<TcpStream>> = vec![];

        loop {
            select! {
                Ok(_)=control_stream.read_u8()=>{
                    for _ in 0..waiting_worker.len(){
                        let _=control_stream.write_u8(0).await;
                    }
                    interval.reset();
                }
                _=interval.tick()=>{
                    println!("Disconnected");
                    break;
                }

                Ok((tcp_stream, addr)) = tcp_listenser.accept() => {
                    if addr.ip() != server_addr.ip() {
                        continue;
                    }
                    if let Some(worker_tx) = waiting_worker.pop() {
                        let _ = worker_tx.send(tcp_stream).await;
                    }
                }

                Some(worker_rx)=manager_rx.recv()=>{
                    waiting_worker.push(worker_rx);
                    let _=control_stream.write_u8(0).await;
                }
            }
        }
    };

    // Spawn a task to handle incoming packets
    let udp_packet_handler = async move {
        let mut buf = [0; 4096];
        loop {
            select! {


                //remove stale mapping
                Some(addr)=stale_rx.recv()=>{
                    addr2handler.remove(&addr);
                }

                Ok((size, addr))= udp_in.recv_from(&mut buf) => {
                    let addr=addr.to_string();
                    let mut pkt=vec![((size>>8) & 0xFF) as u8,(size & 0xFF) as u8];
                    pkt.extend(&buf[..size]);

                    if let Some(tx)=addr2handler.get(&addr){
                        let _=tx.send(pkt).await;
                    }else{
                        let (tx, rx) = tokio::sync::mpsc::channel(4);
                        let _=tx.send(pkt).await;
                        addr2handler.insert(addr.clone(), tx);

                        let (tcp_tx,tcp_rx)=tokio::sync::mpsc::channel(1);
                        let _=manager_tx.send(tcp_tx).await;

                        let udp_stream=udp_listener.clone();
                        let stale_tx=stale_tx.clone();

                        tokio::spawn(async move {

                            let _=handle(addr.clone(), tcp_rx, udp_stream,rx).await;
                            let _ = stale_tx.send(addr).await;
                        });

                    }

                }
            }
        }
    };

    pin_mut!(manager);
    pin_mut!(udp_packet_handler);
    futures::future::select(manager, udp_packet_handler).await;
    Ok(())
}

async fn handle(
    addr: String,
    mut tcp_stream: tokio::sync::mpsc::Receiver<TcpStream>,
    udp_tx: Arc<UdpSocket>,
    mut udp_rx: tokio::sync::mpsc::Receiver<Vec<u8>>,
) -> io::Result<()> {
    //create a new tcp tunnel
    let stream = tcp_stream
        .recv()
        .await
        .ok_or_else(|| "")
        .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;

    let (mut tcp_in, mut tcp_out) = stream.into_split();

    //Address Handshake
    let target = &REMOTE_ADDR;
    let _ = tcp_out.write_u8(target.len() as u8).await;
    tcp_out.write_all(target.as_bytes()).await?;

    println!("Create a new tcp tunnel");
    let mut udp_out = udp_tx.clone();

    let udp2tcp = async move {
        //set a timeout event
        let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(300));

        loop {
            select! {
                Some(data)=udp_rx.recv()=>{
                    let result=tcp_out.write_all(&data).await;
                    if result.is_ok(){
                        //reset timeout
                        interval.reset();
                    }
                }
                //timeout event
                _=interval.tick()=>{
                    break;
                }
            }
        }
    };

    let tcp2udp = async move {
        let mut buf = CircularBuffer::new();
        loop {
            let result = read_tcp_to_buf(&mut tcp_in, &mut buf).await;
            let _ = send_udp_from_buf(&mut udp_out, &mut buf, &addr.clone()).await;

            match result {
                Ok(0) => {
                    break;
                }
                Err(_) => {
                    break;
                }
                Ok(_) => {}
            }
        }
        //tcp close here
    };

    pin_mut!(udp2tcp);
    pin_mut!(tcp2udp);
    futures::future::select(udp2tcp, tcp2udp).await;
    Ok(())
}
