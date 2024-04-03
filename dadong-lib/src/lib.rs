use tokio::io::AsyncReadExt;
use tokio::net::UdpSocket;

use std::sync::Arc;

use tokio::net::tcp::OwnedReadHalf;

const BUFFER_SIZE: usize = 16384;
pub struct CircularBuffer {
    buffer: [u8; BUFFER_SIZE],
    head: usize,
    tail: usize,
    length: usize,
}

impl CircularBuffer {
    //init the buffer
    pub fn new() -> Self {
        CircularBuffer {
            buffer: [0; BUFFER_SIZE],
            head: 0,
            tail: 0,
            length: 0,
        }
    }

    //we assume all pkt have at least length 1
    pub fn first_pkt_length(&self) -> Option<u16> {
        let ptr = self.head;
        if self.length < 2 {
            return None;
        }
        //get packet length from head of buf
        let length = (self.buffer[ptr] as u16) << 8;
        let ptr = ptr + 1;
        let ptr = if ptr >= BUFFER_SIZE { 0 } else { ptr };
        let length = (self.buffer[ptr]) as u16 + length;
        Some(length)
    }
}

const PKT_HEAD_LENGTH: usize = 2;

pub async fn send_udp_from_buf(
    udp: &mut Arc<UdpSocket>,
    buf: &mut CircularBuffer,
    addr: &str,
) -> std::io::Result<()> {
    loop {
        let pkt_len = buf.first_pkt_length();
        if let Some(len) = pkt_len {
            let len = len as usize;
            if buf.length >= len + PKT_HEAD_LENGTH {
                if buf.head + len + PKT_HEAD_LENGTH <= BUFFER_SIZE {
                    let end = buf.head + len + PKT_HEAD_LENGTH;
                    let new_head = if end >= BUFFER_SIZE {
                        end - BUFFER_SIZE
                    } else {
                        end
                    };
                    let _ = udp
                        .send_to(&buf.buffer[buf.head + PKT_HEAD_LENGTH..end], addr)
                        .await;
                    buf.head = new_head;
                } else {
                    //across boundary
                    let new_head = buf.head + len + PKT_HEAD_LENGTH - BUFFER_SIZE;
                    let start = buf.head + PKT_HEAD_LENGTH;
                    let start = if start > BUFFER_SIZE {
                        BUFFER_SIZE
                    } else {
                        start
                    };
                    let pkt = [&buf.buffer[start..BUFFER_SIZE], &buf.buffer[..new_head]].concat();
                    let _ = udp.send_to(&pkt, addr).await;
                    buf.head = new_head;
                }
                buf.length -= len + PKT_HEAD_LENGTH;
            } else {
                break;
            }
        } else {
            break;
        }
    }
    Ok(())
}

pub async fn read_tcp_to_buf(
    tcp_stream: &mut OwnedReadHalf,
    buf: &mut CircularBuffer,
) -> std::io::Result<u16> {
    let mut size = 0;

    if buf.tail >= buf.head {
        let end = if buf.head == 0 {
            BUFFER_SIZE - 1
        } else {
            BUFFER_SIZE
        };
        let read_bytes = tcp_stream
            .read(&mut buf.buffer[(buf.tail as usize)..end])
            .await?;
        buf.tail += read_bytes;
        size += read_bytes;
    }

    //use bit wise and
    if buf.tail >= BUFFER_SIZE {
        buf.tail = 0;
    }

    if buf.tail < buf.head {
        let read_bytes = tcp_stream
            .read(&mut buf.buffer[buf.tail..buf.head - 1])
            .await?;
        buf.tail += read_bytes;
        size += read_bytes;
    };

    buf.length += size;
    Ok(size as u16)
}
