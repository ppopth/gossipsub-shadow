use async_std::stream;
use futures::{prelude::*, select};
use libp2p::gossipsub::MessageId;
use libp2p::gossipsub::{
    Gossipsub, GossipsubEvent, GossipsubMessage, IdentTopic as Topic, MessageAuthenticity,
    ValidationMode,
};
use libp2p::{
    gossipsub, identity, swarm::SwarmEvent, Multiaddr, PeerId, Swarm,
};
use libp2p::multiaddr::Protocol;
use rand::prelude::*;
use rand::Rng;
use std::collections::hash_map::DefaultHasher;
use std::error::Error;
use std::hash::{Hash, Hasher};
use std::net::SocketAddr;
use std::net::ToSocketAddrs;
use std::time::Duration;
use chrono::Local;

fn log(s: String) {
    println!("{}: {}", Local::now(), s);
}

#[async_std::main]
async fn main() -> Result<(), Box<dyn Error>> {
    // Create a random PeerId
    let local_key = identity::Keypair::generate_ed25519();
    let local_peer_id = PeerId::from(local_key.public());
    log(format!("Local peer id: {local_peer_id}"));

    // Set up an encrypted DNS-enabled TCP Transport over the Mplex protocol.
    let transport = libp2p::development_transport(local_key.clone()).await?;

    // To content-address message, we can take the hash of message and use it as an ID.
    let message_id_fn = |message: &GossipsubMessage| {
        let mut s = DefaultHasher::new();
        message.data.hash(&mut s);
        MessageId::from(s.finish().to_string())
    };

    // Set a custom gossipsub configuration
    let gossipsub_config = gossipsub::GossipsubConfigBuilder::default()
        .heartbeat_interval(Duration::from_secs(10)) // This is set to aid debugging by not cluttering the log space
        .validation_mode(ValidationMode::Strict) // This sets the kind of message validation. The default is Strict (enforce message signing)
        .message_id_fn(message_id_fn) // content-address messages. No two messages of the same content will be propagated.
        .build()
        .expect("Valid config");

    // build a gossipsub network behaviour
    let mut gossipsub: Gossipsub = Gossipsub::new(MessageAuthenticity::Signed(local_key), gossipsub_config)
        .expect("Correct configuration");

    // Create a Gossipsub topic
    let topic = Topic::new("test-net");

    // subscribes to our topic
    gossipsub.subscribe(&topic)?;

    // Create a Swarm to manage peers and events
    let mut swarm = Swarm::with_async_std_executor(transport, gossipsub, local_peer_id);

    // Listen on all interfaces and whatever port the OS assigns
    swarm.listen_on("/ip4/0.0.0.0/tcp/8080".parse()?)?;

    // Create the random number generator
    let mut rng = rand::thread_rng();
    let mut interval = stream::interval(Duration::from_secs(10)).fuse();

    let hostname = nix::unistd::gethostname().unwrap().into_string().unwrap();
    let host_idx = hostname[4..].parse().unwrap();
    let num_peers: u16 = std::env::args().nth(1).unwrap().parse().unwrap();
    let mut nums: Vec<u16> = (1..=num_peers).collect();
    nums.shuffle(&mut rng);
    let mut idx = 0;
    let mut connected = 0;
    while connected < 6 {
        let addr = format!("{}{}", "peer", nums[idx]);
        if addr == hostname {
            idx += 1;
            continue;
        }
        if nums[idx] < host_idx {
            idx += 1;
            connected += 1;
            continue;
        }
        log(format!("connecting to {} from peer{}", &addr, host_idx));
        let SocketAddr::V4(sock_addr) = (&addr[..], 8080).to_socket_addrs()?.next().unwrap() else { todo!() };

        let mut remote: Multiaddr = Multiaddr::empty();
        remote.push(Protocol::Ip4(*sock_addr.ip()));
        remote.push(Protocol::Tcp(sock_addr.port()));
        swarm.dial(remote)?;
        connected += 1;
        idx += 1;
    }

    // Kick it off
    loop {
        select! {
            () = interval.select_next_some() => {
                if host_idx == 1 {
                    if let Err(e) = swarm
                        .behaviour_mut()
                        .publish(topic.clone(), format!("Hello {:08x} from {}", rng.gen::<u32>(), local_peer_id)) {
                        log(format!("Publish error: {e:?}"));
                    }
                }
            },
            event = swarm.select_next_some() => match event {
                SwarmEvent::Behaviour(GossipsubEvent::Message {
                    propagation_source: _peer_id,
                    message_id: _id,
                    message,
                }) => log(format!(
                        "Got message: '{}'",
                        String::from_utf8_lossy(&message.data),
                    )),
                _ => {}
            }
        }
    }
}
