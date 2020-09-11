use std::{
    fmt,
    net::SocketAddr,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};

use flume::{unbounded, Receiver};
use futures_util::{future::BoxFuture, ready, stream::Stream};

use crate::{error::*, packet::Packet, router::Router, socket::UtpSocket};

// TODO: Need to figure out a plan to deal with lost packets: one idea is to have a queue
// of unacked packets, pass a reference into the write future, and access the queue from
// the future... something like that
pub struct Connection {
    socket: Arc<UtpSocket>,
    connection_id: u16,
    remote_addr: SocketAddr,
    router: Arc<Router>,
    packet_rx: Receiver<(Packet, SocketAddr)>,
    read_future: Option<BoxFuture<'static, Result<(Packet, SocketAddr)>>>,
    write_future: Option<BoxFuture<'static, Result<usize>>>,
}

impl Connection {
    pub fn new(
        socket: Arc<UtpSocket>,
        connection_id: u16,
        remote_addr: SocketAddr,
        router: Arc<Router>,
        packet_rx: Receiver<(Packet, SocketAddr)>,
        read_future: Option<BoxFuture<'static, Result<(Packet, SocketAddr)>>>,
        write_future: Option<BoxFuture<'static, Result<usize>>>,
    ) -> Self {
        Self {
            socket,
            connection_id,
            remote_addr,
            router,
            packet_rx,
            read_future,
            write_future,
        }
    }

    pub fn generate(
        socket: Arc<UtpSocket>,
        router: Arc<Router>,
        remote_addr: SocketAddr,
    ) -> Result<Self> {
        let (packet_tx, packet_rx) = unbounded();

        Ok(Self::new(
            socket,
            router.register_channel(packet_tx)?,
            remote_addr,
            router,
            packet_rx,
            None,
            None, // TODO: Write SYN packet to remote socket
        ))
    }
}

impl Stream for Connection {
    type Item = Result<()>; // TODO: Add some "message" type

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context) -> Poll<Option<Self::Item>> {
        // Do not progress until any writes to the socket have finished.
        if self.write_future.is_some() {
            // TODO: Handle this result in case it failed
            let _ = ready!(self.write_future.as_mut().unwrap().as_mut().poll(cx));
            // Remove the future if it finished
            self.write_future.take();
        }

        // Now there are guaranteed to be no pending writes, so check for incoming packets.
        let result = if let Ok(packet_and_addr) = self.packet_rx.try_recv() {
            Ok(packet_and_addr)
        } else if self.read_future.is_some() {
            let packet_and_addr = ready!(self.read_future.as_mut().unwrap().as_mut().poll(cx));
            // Remove the future if it finished
            self.read_future.take();
            packet_and_addr
        } else {
            let socket = Arc::clone(&self.socket);
            self.read_future = Some(Box::pin(async move { socket.recv_from().await }));
            let packet_and_addr = ready!(self.read_future.as_mut().unwrap().as_mut().poll(cx));
            // Remove the future if it finished
            self.read_future.take();
            packet_and_addr
        };

        match result {
            Ok((packet, addr)) => {
                if packet.connection_id != self.connection_id {
                    // This packet isn't meant for us
                    self.router.route(packet, addr);
                    return Poll::Pending;
                }

                if self.remote_addr != addr {
                    // Somehow we got this packet from an unfamiliar address
                    // TODO: Log this event and drop the packet?
                }

                println!("Connection {} got packet: {:?}", self.connection_id, packet);
                todo!()
            }
            Err(err) => Poll::Ready(Some(Err(err))),
        }
    }
}

impl fmt::Debug for Connection {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_fmt(format_args!(
            "Connection {{ connection_id: {}, remote_addr: {} }}",
            self.connection_id, self.remote_addr
        ))
    }
}
