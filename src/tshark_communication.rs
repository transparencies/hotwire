use crate::http::tshark_http;
use crate::http2::tshark_http2;
use crate::pgsql::tshark_pgsql;
use chrono::NaiveDateTime;
use quick_xml::events::Event;
use std::fmt::Debug;
use std::io::BufRead;
use std::str;
use std::str::FromStr;

#[derive(Debug)]
pub struct TSharkPacketBasicInfo {
    pub frame_time: NaiveDateTime,
    pub ip_src: String, // v4 or v6
    pub ip_dst: String, // v4 or v6
    pub tcp_seq_number: u32,
    pub tcp_stream_id: u32,
    pub port_src: u32,
    pub port_dst: u32,
}

#[derive(Debug)]
pub struct TSharkPacket {
    pub basic_info: TSharkPacketBasicInfo,
    pub http: Option<tshark_http::TSharkHttp>,
    pub http2: Option<Vec<tshark_http2::TSharkHttp2Message>>,
    pub pgsql: Option<Vec<tshark_pgsql::PostgresWireMessage>>,
}

pub fn parse_packet<B: BufRead>(
    xml_reader: &mut quick_xml::Reader<B>,
    buf: &mut Vec<u8>,
) -> Result<TSharkPacket, quick_xml::Error> {
    let mut frame_time = NaiveDateTime::from_timestamp(0, 0);
    let mut ip_src = None;
    let mut ip_dst = None;
    let mut tcp_seq_number = 0;
    let mut tcp_stream_id = 0;
    let mut port_src = 0;
    let mut port_dst = 0;
    let mut http = None;
    let mut http2 = None;
    let mut pgsql = None::<Vec<tshark_pgsql::PostgresWireMessage>>;
    loop {
        match xml_reader.read_event(buf) {
            Ok(Event::Start(ref e)) => {
                if e.name() == b"proto" {
                    let name = e
                        .attributes()
                        .find(|kv| kv.as_ref().unwrap().key == "name".as_bytes())
                        .map(|kv| kv.unwrap().value);
                    match name.as_deref() {
                        Some(b"frame") => {
                            frame_time = parse_frame_info(xml_reader, buf);
                        }
                        Some(b"ip") => {
                            let ip_info = parse_ip_info(xml_reader, buf);
                            ip_src = Some(ip_info.0);
                            ip_dst = Some(ip_info.1);
                        }
                        // TODO ipv6
                        Some(b"tcp") => {
                            // waiting for https://github.com/rust-lang/rust/issues/71126
                            let tcp_info = parse_tcp_info(xml_reader, buf);
                            tcp_seq_number = tcp_info.0;
                            tcp_stream_id = tcp_info.1;
                            port_src = tcp_info.2;
                            port_dst = tcp_info.3;
                        }
                        Some(b"http") => {
                            http = Some(tshark_http::parse_http_info(xml_reader, buf));
                        }
                        Some(b"http2") => {
                            http2 = Some(tshark_http2::parse_http2_info(xml_reader, buf));
                        }
                        Some(b"pgsql") => {
                            let mut pgsql_packets = tshark_pgsql::parse_pgsql_info(xml_reader, buf);
                            if let Some(mut sofar) = pgsql {
                                sofar.append(&mut pgsql_packets);
                                pgsql = Some(sofar);
                            } else {
                                pgsql = Some(pgsql_packets);
                            }
                        }
                        _ => {}
                    }
                }
            }
            Ok(Event::End(ref e)) => {
                if e.name() == b"packet" {
                    return Ok(TSharkPacket {
                        basic_info: TSharkPacketBasicInfo {
                            frame_time,
                            ip_src: ip_src.unwrap_or_default(),
                            ip_dst: ip_dst.unwrap_or_default(),
                            tcp_seq_number,
                            tcp_stream_id,
                            port_src,
                            port_dst,
                        },
                        http,
                        http2,
                        pgsql,
                    });
                }
            }
            Err(e) => return Err(e),
            _ => {}
        }
        // buf.clear();
    }
}

fn parse_frame_info<B: BufRead>(
    xml_reader: &mut quick_xml::Reader<B>,
    buf: &mut Vec<u8>,
) -> NaiveDateTime {
    loop {
        match xml_reader.read_event(buf) {
            Ok(Event::Empty(ref e)) => {
                if e.name() == b"field"
                    && e.attributes().any(|kv| {
                        kv.unwrap() == ("name".as_bytes(), "frame.time".as_bytes()).into()
                    })
                {
                    // dbg!(e);
                    // panic!();
                    if let Some(time_str) = e.attributes().find_map(|a| {
                        Some(a.unwrap())
                            .filter(|a| a.key == b"show")
                            .map(|a| String::from_utf8(a.value.to_vec()).unwrap())
                    }) {
                        // must use NaiveDateTime because chrono can't read string timezone names.
                        // https://docs.rs/chrono/0.4.19/chrono/format/strftime/index.html#specifiers
                        // > %Z: Offset will not be populated from the parsed data, nor will it be validated.
                        // > Timezone is completely ignored. Similar to the glibc strptime treatment of this format code.
                        // > It is not possible to reliably convert from an abbreviation to an offset, for example CDT
                        // > can mean either Central Daylight Time (North America) or China Daylight Time.
                        return NaiveDateTime::parse_from_str(&time_str, "%b %e, %Y %T.%f %Z")
                            .unwrap();
                    }
                }
            }
            _ => {}
        }
    }
}

fn parse_ip_info<B: BufRead>(
    xml_reader: &mut quick_xml::Reader<B>,
    buf: &mut Vec<u8>,
) -> (String, String) {
    let mut ip_src = None;
    let mut ip_dst = None;
    loop {
        match xml_reader.read_event(buf) {
            Ok(Event::Empty(ref e)) => {
                if e.name() == b"field" {
                    let name = e
                        .attributes()
                        .find(|kv| kv.as_ref().unwrap().key == "name".as_bytes())
                        .map(|kv| kv.unwrap().value);
                    match name.as_deref() {
                        Some(b"ip.src") => {
                            ip_src = element_attr_val_string(e, b"show");
                        }
                        Some(b"ip.dst") => {
                            ip_dst = element_attr_val_string(e, b"show");
                        }
                        _ => {}
                    }
                }
            }
            Ok(Event::End(ref e)) => {
                if e.name() == b"proto" {
                    return (ip_src.unwrap_or_default(), ip_dst.unwrap_or_default());
                }
            }
            _ => {}
        }
    }
}

fn parse_tcp_info<B: BufRead>(
    xml_reader: &mut quick_xml::Reader<B>,
    buf: &mut Vec<u8>,
) -> (u32, u32, u32, u32) {
    let mut tcp_seq_number = 0;
    let mut tcp_stream_id = 0;
    let mut port_src = 0;
    let mut port_dst = 0;
    loop {
        match xml_reader.read_event(buf) {
            Ok(Event::Empty(ref e)) => {
                if e.name() == b"field" {
                    let name = e
                        .attributes()
                        .find(|kv| kv.as_ref().unwrap().key == "name".as_bytes())
                        .map(|kv| kv.unwrap().value);
                    match name.as_deref() {
                        Some(b"tcp.srcport") => {
                            port_src = element_attr_val_number(e, b"show").unwrap();
                        }
                        Some(b"tcp.dstport") => {
                            port_dst = element_attr_val_number(e, b"show").unwrap();
                        }
                        Some(b"tcp.seq_raw") => {
                            tcp_seq_number = element_attr_val_number(e, b"show").unwrap();
                        }
                        Some(b"tcp.stream") => {
                            tcp_stream_id = element_attr_val_number(e, b"show").unwrap();
                        }
                        _ => {}
                    }
                }
            }
            Ok(Event::End(ref e)) => {
                if e.name() == b"proto" {
                    return (tcp_seq_number, tcp_stream_id, port_src, port_dst);
                }
            }
            _ => {}
        }
    }
}

pub fn element_attr_val_number<'a, F: FromStr>(
    e: &'a quick_xml::events::BytesStart<'a>,
    attr_name: &'static [u8],
) -> Option<F>
where
    <F as FromStr>::Err: Debug,
{
    str::from_utf8(
        e.attributes()
            .find(|kv| kv.as_ref().unwrap().key == attr_name)
            .unwrap()
            .unwrap()
            .unescaped_value()
            .unwrap()
            .as_ref(),
    )
    .unwrap()
    .parse()
    .ok()
}

pub fn element_attr_val_string<'a>(
    e: &'a quick_xml::events::BytesStart<'a>,
    attr_name: &'static [u8],
) -> Option<String> {
    String::from_utf8(
        e.attributes()
            .find(|kv| kv.as_ref().unwrap().key == attr_name)
            .unwrap()
            .unwrap()
            .unescaped_value()
            .unwrap()
            .to_vec(),
    )
    .ok()
}
