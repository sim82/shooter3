use std::{
    collections::BTreeSet,
    fs::File,
    io::{BufRead, BufReader},
};

use bevy::utils::HashMap;
use renet_test::controller::{ExternalLogRecord, FrameTime};

fn main() {
    let client_records = to_map(read_records("client.log"));
    let server_records = to_map(read_records("server.log"));

    let cs = client_records.keys().cloned().collect::<BTreeSet<_>>();
    let ss = server_records.keys().cloned().collect::<BTreeSet<_>>();

    let common_serials = cs.intersection(&ss);

    for serial in common_serials {
        let client_record = client_records.get(serial).unwrap(); // must succeed
        let server_record = server_records.get(serial).unwrap(); // must succeed
        let delta = client_record.pos - server_record.pos;
        let delta_len = delta.length();

        println!(
            "{} {:?} {} {}",
            serial,
            delta_len,
            FrameTime::new(client_record.dt),
            FrameTime::new(server_record.dt)
        )
    }

    // println!("{:?}", records);
}

fn to_map(mut records: Vec<ExternalLogRecord>) -> HashMap<u32, ExternalLogRecord> {
    records
        .drain(..)
        .map(|r| (r.serial, r))
        .collect::<HashMap<_, _>>()
}

fn read_records(path: &str) -> Vec<ExternalLogRecord> {
    let file = BufReader::new(File::open(path).unwrap());
    let records: Vec<ExternalLogRecord> = file
        .lines()
        .filter_map(|line| line.ok())
        .filter_map(|line| {
            serde_json::from_str::<renet_test::controller::ExternalLogRecord>(&line).ok()
        })
        .collect();
    records
}
