use clap::{Parser, Subcommand};
use std::net::{Ipv6Addr, Ipv4Addr, IpAddr};
use softnpu_standalone::mgmt::{
    ManagementRequest, ManagementResponse, TableAdd, TableRemove,
};
use tokio::net::UnixDatagram;
use p4rs::TableEntry;
use std::collections::BTreeMap;

#[derive(Parser, Debug)]
#[clap(version, about)]
struct Cli {
    #[clap(subcommand)]
    command: Commands,
}

#[derive(Debug, Subcommand)]
enum Commands {
    /// Add an IPv4 route to the routing table.
    AddRoute4 {
        /// Destination address for the route.
        destination: Ipv4Addr,

        /// Subnet mask for the destination.
        mask: u8,

        /// Outbound port for the route.
        port: u8,

        /// Next Hop
        nexthop: Ipv4Addr,
    },

    /// Remove a route from the routing table.
    RemoveRoute4 {
        /// Destination address for the route.
        destination: Ipv4Addr,

        /// Subnet mask for the destination.
        mask: u8,
    },

    /// Add an IPv6 route to the routing table.
    AddRoute6 {
        /// Destination address for the route.
        destination: Ipv6Addr,

        /// Subnet mask for the destination.
        mask: u8,

        /// Outbound port for the route.
        port: u8,

        /// Next Hop
        nexthop: Ipv6Addr,
    },

    /// Remove a route from the routing table.
    RemoveRoute6 {
        /// Destination address for the route.
        destination: Ipv6Addr,

        /// Subnet mask for the destination.
        mask: u8,
    },

    /// Add an IPv6 address to the router.
    AddAddress6 {
        /// Address to add.
        address: Ipv6Addr,
    },

    /// Remove an IPv6 address from the router.
    RemoveAddress6 {
        /// Address to add.
        address: Ipv6Addr,
    },

    /// Add an IPv4 address to the router.
    AddAddress4 {
        /// Address to add.
        address: Ipv4Addr,
    },

    /// Remove an IPv4 address from the router.
    RemoveAddress4 {
        /// Address to add.
        address: Ipv4Addr,
    },

    /// Specify MAC address for a port.
    SetMac {
        /// Port to set MAC for.
        port: u8,
        /// The MAC address.
        mac: MacAddr,
    },

    /// Clear a port's MAC address.
    ClearMac { port: u8 },

    /// Show port count
    PortCount,

    /// Add a static NDP entry
    AddNdpEntry { l3: Ipv6Addr, l2: MacAddr },

    /// Remove a static NDP entry
    RemoveNdpEntry { l3: Ipv6Addr },

    /// Add a static ARP entry
    AddArpEntry { l3: Ipv4Addr, l2: MacAddr },

    /// Remove a static ARP entry
    RemoveArpEntry { l3: Ipv4Addr },

    /// Dump all tables
    DumpState,

    /// Add an IPv6 NAT entry
    AddNat6 {
        /// Destination address for ingress NAT packets.
        dst: Ipv6Addr,
        /// Beginning of L4 port range for this entry.
        begin: u16,
        /// End of L4 port range for this entry.
        end: u16,
        /// Underlay IPv6 address to send encapsulated packets to.
        target: Ipv6Addr,
        /// VNI to encapsulate packets onto.
        vni: u32,
        /// Mac address to use for inner-packet L2 destination.
        mac: MacAddr,
    },

    /// Remove an IPv6 NAT entry
    RemoveNat6 { dst: Ipv6Addr, begin: u16, end: u16 },

    /// Add an IPv4 NAT entry
    AddNat4 {
        /// Destination address for ingress NAT packets.
        dst: Ipv4Addr,
        /// Beginning of L4 port range for this entry.
        begin: u16,
        /// End of L4 port range for this entry.
        end: u16,
        /// Underlay IPv6 address to send encapsulated packets to.
        target: Ipv6Addr,
        /// VNI to encapsulate packets onto.
        vni: u32,
        /// Mac address to use for inner-packet L2 destination.
        mac: MacAddr,
    },

    /// Remove an IPv4 NAT entry
    RemoveNat4 { dst: Ipv4Addr, begin: u16, end: u16 },

    /// Add a proxy ARP entry.
    AddProxyArp { begin: Ipv4Addr, end: Ipv4Addr, mac: MacAddr },

    /// Remove a proxy ARP entry.
    RemoveProxyArp { begin: Ipv4Addr, end: Ipv4Addr },
}

#[derive(Debug, Clone)]
struct MacAddr(pub [u8; 6]);

impl std::str::FromStr for MacAddr {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let parts: Vec<&str> = s.split(':').collect();
        if parts.len() != 6 {
            return Err("Expected mac in the form aa:bb:cc:dd:ee:ff".into());
        }
        let mut result = MacAddr([0u8; 6]);
        for (i, p) in parts.iter().enumerate() {
            result.0[i] = match u8::from_str_radix(p, 16) {
                Ok(n) => n,
                Err(_) => {
                    return Err(
                        "Expected mac in the form aa:bb:cc:dd:ee:ff".into()
                    );
                }
            }
        }
        Ok(result)
    }
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();

    match cli.command {
        Commands::AddRoute4 { destination, mask, port, nexthop } => {
            let mut keyset_data: Vec<u8> = destination.octets().into();
            keyset_data.push(mask);

            let mut parameter_data = vec![port];
            let nexthop_data: Vec<u8> = nexthop.octets().into();
            parameter_data.extend_from_slice(&nexthop_data);

            send(ManagementRequest::TableAdd(TableAdd {
                table: 3,
                action: 1,
                keyset_data,
                parameter_data,
            })).await;
        }

        Commands::RemoveRoute4 { destination, mask } => {
            let mut keyset_data: Vec<u8> = destination.octets().into();
            keyset_data.push(mask);

            send(ManagementRequest::TableRemove(TableRemove {
                table: 3,
                keyset_data,
            })).await;
        }

        Commands::AddRoute6 { destination, mask, port, nexthop } => {
            let mut keyset_data: Vec<u8> = destination.octets().into();
            keyset_data.push(mask);

            let mut parameter_data = vec![port];
            let nexthop_data: Vec<u8> = nexthop.octets().into();
            parameter_data.extend_from_slice(&nexthop_data);

            send(ManagementRequest::TableAdd(TableAdd {
                table: 2,
                action: 1,
                keyset_data,
                parameter_data,
            })).await;
        }

        Commands::RemoveRoute6 { destination, mask } => {
            let mut keyset_data: Vec<u8> = destination.octets().into();
            keyset_data.push(mask);

            send(ManagementRequest::TableRemove(TableRemove {
                table: 2,
                keyset_data,
            })).await;
        }

        Commands::AddNat4 { dst, begin, end, target, vni, mac } => {
            if vni >= 1 << 24 {
                println!("vni too big, only 24 bits");
                std::process::exit(1);
            }

            let mut keyset_data: Vec<u8> = dst.octets().into();
            keyset_data.extend_from_slice(&begin.to_be_bytes());
            keyset_data.extend_from_slice(&end.to_be_bytes());

            let mut parameter_data: Vec<u8> = target.octets().into();
            let vni_bits = vni.to_be_bytes();
            parameter_data.extend_from_slice(&vni_bits[1..4]);
            parameter_data.extend_from_slice(&mac.0);

            send(ManagementRequest::TableAdd(TableAdd {
                table: 4,
                action: 0,
                keyset_data,
                parameter_data,
            })).await;
        }

        Commands::RemoveNat4 { dst, begin, end } => {
            let mut keyset_data: Vec<u8> = dst.octets().into();
            keyset_data.extend_from_slice(&begin.to_be_bytes());
            keyset_data.extend_from_slice(&end.to_be_bytes());

            send(ManagementRequest::TableRemove(TableRemove {
                table: 4,
                keyset_data,
            })).await;
        }

        Commands::AddNat6 { dst, begin, end, target, vni, mac } => {
            if vni >= 1 << 24 {
                println!("vni too big, only 24 bits");
                std::process::exit(1);
            }

            let mut keyset_data: Vec<u8> = dst.octets().into();
            keyset_data.extend_from_slice(&begin.to_be_bytes());
            keyset_data.extend_from_slice(&end.to_be_bytes());

            let mut parameter_data: Vec<u8> = target.octets().into();
            let vni_bits = vni.to_be_bytes();
            parameter_data.extend_from_slice(&vni_bits[1..4]);
            parameter_data.extend_from_slice(&mac.0);

            send(ManagementRequest::TableAdd(TableAdd {
                table: 5,
                action: 0,
                keyset_data,
                parameter_data,
            })).await;
        }

        Commands::RemoveNat6 { dst, begin, end } => {
            let mut keyset_data: Vec<u8> = dst.octets().into();
            keyset_data.extend_from_slice(&begin.to_be_bytes());
            keyset_data.extend_from_slice(&end.to_be_bytes());

            send(ManagementRequest::TableRemove(TableRemove {
                table: 5,
                keyset_data,
            })).await;
        }

        Commands::AddAddress4 { address } => {
            let keyset_data: Vec<u8> = address.octets().into();
            send(ManagementRequest::TableAdd(TableAdd {
                table: 1,
                action: 0,
                keyset_data,
                ..Default::default()
            })).await;
        }

        Commands::RemoveAddress4 { address } => {
            let keyset_data: Vec<u8> = address.octets().into();
            send(ManagementRequest::TableRemove(TableRemove {
                table: 1,
                keyset_data,
            })).await;
        }

        Commands::AddAddress6 { address } => {
            let keyset_data: Vec<u8> = address.octets().into();
            send(ManagementRequest::TableAdd(TableAdd {
                table: 0,
                action: 0,
                keyset_data,
                ..Default::default()
            })).await;
        }

        Commands::RemoveAddress6 { address } => {
            let keyset_data: Vec<u8> = address.octets().into();
            send(ManagementRequest::TableRemove(TableRemove {
                table: 0,
                keyset_data,
            })).await;
        }

        Commands::SetMac { port, mac } => {
            let keyset_data: Vec<u8> = vec![port];
            let parameter_data: Vec<u8> = mac.0.into();
            send(ManagementRequest::TableAdd(TableAdd {
                table: 10,
                action: 0,
                keyset_data,
                parameter_data,
            })).await;
        }

        Commands::ClearMac { port } => {
            let keyset_data: Vec<u8> = vec![port];
            send(ManagementRequest::TableRemove(TableRemove {
                table: 10,
                keyset_data,
            })).await;
        }

        Commands::PortCount => {
            let uds = bind();
            let j = tokio::spawn(async move { 
                match recv(uds).await {
                    ManagementResponse::RadixResponse(n) => println!("{}", n),
                    _ => {}
                }});
            send(ManagementRequest::RadixRequest).await;
            j.await.unwrap();
        }

        Commands::AddNdpEntry { l3, l2 } => {
            let keyset_data: Vec<u8> = l3.octets().into();
            let parameter_data: Vec<u8> = l2.0.into();
            send(ManagementRequest::TableAdd(TableAdd {
                table: 9,
                action: 0,
                keyset_data,
                parameter_data,
            })).await;
        }

        Commands::RemoveNdpEntry { l3 } => {
            let keyset_data: Vec<u8> = l3.octets().into();
            send(ManagementRequest::TableRemove(TableRemove {
                table: 9,
                keyset_data,
            })).await;
        }

        Commands::AddArpEntry { l3, l2 } => {
            let keyset_data: Vec<u8> = l3.octets().into();
            let parameter_data: Vec<u8> = l2.0.into();
            send(ManagementRequest::TableAdd(TableAdd {
                table: 8,
                action: 0,
                keyset_data,
                parameter_data,
            })).await;
        }

        Commands::RemoveArpEntry { l3 } => {
            let keyset_data: Vec<u8> = l3.octets().into();
            send(ManagementRequest::TableRemove(TableRemove {
                table: 8,
                keyset_data,
            })).await;
        }

        Commands::DumpState => {
            let uds = bind();
            let j = tokio::spawn(async move { 
                match recv(uds).await {
                    ManagementResponse::DumpResponse(ref tables) => {
                        dump_tables(tables);
                    }
                    _ => {}
                }});
            send(ManagementRequest::DumpRequest).await;
            j.await.unwrap();
        }

        Commands::AddProxyArp { begin, end, mac } => {
            let mut keyset_data: Vec<u8> = begin.octets().into();
            keyset_data.extend_from_slice(&end.octets());

            let parameter_data: Vec<u8> = mac.0.into();

            send(ManagementRequest::TableAdd(TableAdd {
                table: 11,
                action: 0,
                keyset_data,
                parameter_data,
            })).await;
        }

        Commands::RemoveProxyArp { begin, end } => {
            let mut keyset_data: Vec<u8> = begin.octets().into();
            keyset_data.extend_from_slice(&end.octets());

            send(ManagementRequest::TableRemove(TableRemove {
                table: 11,
                keyset_data,
            })).await;
        }
    }
}

const SERVER: &str = "/opt/scrimlet/stuff/server";
const CLIENT: &str = "/opt/scrimlet/stuff/client";

async fn send(msg: ManagementRequest) {
    let uds = UnixDatagram::unbound().unwrap();

    let buf = serde_json::to_vec(&msg).unwrap();
    uds.send_to(&buf, SERVER).await.unwrap();
}

fn bind() -> UnixDatagram {
    let _ = std::fs::remove_file(CLIENT);
    UnixDatagram::bind(CLIENT).unwrap()
}

async fn recv(uds: UnixDatagram) -> ManagementResponse {
    let mut buf = vec![0u8; 10240];
    let n = uds.recv(&mut buf).await.unwrap();
    serde_json::from_slice(&buf[..n]).unwrap()
}

fn dump_tables(tables: &BTreeMap<u32, Vec<TableEntry>>) {
    println!("local v6:");
    for e in tables.get(&0).unwrap() {
        if let Some(a) = get_addr(&e.keyset_data) {
            println!("{}", a)
        }
    }
    println!("local v4:");
    for e in tables.get(&1).unwrap() {
        if let Some(a) = get_addr(&e.keyset_data) {
            println!("{}", a)
        }
    }

    println!("router v6:");
    for e in tables.get(&2).unwrap() {
        let tgt = match get_addr_subnet(&e.keyset_data) {
            Some((a, m)) => format!("{}/{}", a, m),
            None => "?".into(),
        };
        let gw = match get_port_addr(&e.parameter_data) {
            Some((a, p)) => format!("{} ({})", a, p),
            None => "?".into(),
        };
        println!("{} -> {}", tgt, gw);
    }
    println!("router v4:");
    for e in tables.get(&3).unwrap() {
        let tgt = match get_addr_subnet(&e.keyset_data) {
            Some((a, m)) => format!("{}/{}", a, m),
            None => "?".into(),
        };
        let gw = match get_port_addr(&e.parameter_data) {
            Some((a, p)) => format!("{} ({})", a, p),
            None => "?".into(),
        };
        println!("{} -> {}", tgt, gw);
    }

    println!("resolver v4:");
    for e in tables.get(&8).unwrap() {
        let l3 = match get_addr(&e.keyset_data) {
            Some(a) => a.to_string(),
            None => "?".into(),
        };
        let l2 = match get_mac(&e.parameter_data) {
            Some(m) => format!(
                "{:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
                m[0], m[1], m[2], m[3], m[4], m[5],
            ),
            None => "?".into(),
        };
        println!("{} -> {}", l3, l2);
    }

    println!("resolver v6:");
    for e in tables.get(&9).unwrap() {
        let l3 = match get_addr(&e.keyset_data) {
            Some(a) => a.to_string(),
            None => "?".into(),
        };
        let l2 = match get_mac(&e.parameter_data) {
            Some(m) => format!(
                "{:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
                m[0], m[1], m[2], m[3], m[4], m[5],
            ),
            None => "?".into(),
        };
        println!("{} -> {}", l3, l2);
    }

    println!("nat_v4:");
    for e in tables.get(&4).unwrap() {
        let dst_nat_id = match get_addr_nat_id(&e.keyset_data) {
            Some((dst, nat_start, nat_end)) => {
                format!("{} {}/{}", dst, nat_start, nat_end,)
            }
            None => "?".into(),
        };
        let target = match get_addr_vni_mac(&e.parameter_data) {
            Some((addr, vni, m)) => format!(
                "{} {}/{:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
                addr, vni, m[0], m[1], m[2], m[3], m[4], m[5],
            ),
            None => "?".into(),
        };
        println!("{} -> {}", dst_nat_id, target);
    }
    println!("nat_v6:");
    for e in tables.get(&5).unwrap() {
        let dst_nat_id = match get_addr_nat_id(&e.keyset_data) {
            Some((dst, nat_start, nat_end)) => {
                format!("{} {}/{}", dst, nat_start, nat_end,)
            }
            None => "?".into(),
        };
        let target = match get_addr_vni_mac(&e.parameter_data) {
            Some((addr, vni, m)) => format!(
                "{} {}/{:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
                addr, vni, m[0], m[1], m[2], m[3], m[4], m[5],
            ),
            None => "?".into(),
        };
        println!("{} -> {}", dst_nat_id, target);
    }

    println!("port_mac:");
    for e in tables.get(&10).unwrap() {
        let port = e.keyset_data[0];

        let m = &e.parameter_data;

        let mac = format!(
            "{:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
            m[0], m[1], m[2], m[3], m[4], m[5],
        );
        println!("{}: {}", port, mac);
    }

    println!("icmp_v6:");
    for e in tables.get(&6).unwrap() {
        dump_table_entry(e);
    }
    println!("icmp_v4:");
    for e in tables.get(&7).unwrap() {
        dump_table_entry(e);
    }

    println!("proxy_arp:");
    for e in tables.get(&11).unwrap() {
        let begin = Ipv4Addr::new(
            e.keyset_data[0],
            e.keyset_data[1],
            e.keyset_data[2],
            e.keyset_data[3],
        );
        let end = Ipv4Addr::new(
            e.keyset_data[4],
            e.keyset_data[5],
            e.keyset_data[6],
            e.keyset_data[7],
        );

        let m = &e.parameter_data;

        let mac = format!(
            "{:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
            m[0], m[1], m[2], m[3], m[4], m[5],
        );
        println!("{}/{}: {}", begin, end, mac);
    }
}

fn dump_table_entry(e: &p4rs::TableEntry) {
    println!("{} {:#x?} {:#x?}", e.action_id, e.keyset_data, e.parameter_data);
}

fn get_addr(data: &[u8]) -> Option<IpAddr> {
    match data.len() {
        4 => {
            let buf: [u8; 4] = data.try_into().unwrap();
            Some(Ipv4Addr::from(buf).into())
        }
        16 => {
            let buf: [u8; 16] = data.try_into().unwrap();
            Some(Ipv6Addr::from(buf).into())
        }
        _ => {
            println!("expected address, found: {:x?}", data);
            None
        }
    }
}

fn get_mac(data: &[u8]) -> Option<[u8; 6]> {
    match data.len() {
        6 => Some(data.try_into().unwrap()),
        _ => {
            println!("expected mac address, found: {:x?}", data);
            None
        }
    }
}

fn get_addr_subnet(data: &[u8]) -> Option<(IpAddr, u8)> {
    match data.len() {
        5 => Some((get_addr(&data[..4])?, data[4])),
        17 => Some((get_addr(&data[..16])?, data[16])),
        _ => {
            println!("expected [address, subnet], found: {:x?}", data);
            None
        }
    }
}

fn get_addr_vni_mac(data: &[u8]) -> Option<(IpAddr, u32, [u8; 6])> {
    match data.len() {
        13 => Some((
            get_addr(&data[..4])?,
            u32::from_be_bytes([0, data[4], data[5], data[6]]),
            data[7..13].try_into().ok()?,
        )),
        25 => Some((
            get_addr(&data[..16])?,
            u32::from_be_bytes([0, data[16], data[17], data[18]]),
            data[19..25].try_into().ok()?,
        )),
        _ => {
            println!("expected [address, vni, mac], found: {:x?}", data);
            None
        }
    }
}

fn get_addr_nat_id(data: &[u8]) -> Option<(IpAddr, u16, u16)> {
    match data.len() {
        8 => Some((
            get_addr(&data[..4])?,
            u16::from_be_bytes([data[4], data[5]]),
            u16::from_be_bytes([data[6], data[7]]),
        )),
        20 => Some((
            get_addr(&data[..16])?,
            u16::from_be_bytes([data[16], data[17]]),
            u16::from_be_bytes([data[18], data[19]]),
        )),
        _ => {
            println!("expected [address, nat_id], found: {:x?}", data);
            None
        }
    }
}

fn get_port_addr(data: &[u8]) -> Option<(IpAddr, u8)> {
    match data.len() {
        5 => Some((get_addr(&data[1..])?, data[0])),
        17 => Some((get_addr(&data[1..])?, data[0])),
        _ => {
            println!("expected [port, address], found: {:x?}", data);
            None
        }
    }
}
