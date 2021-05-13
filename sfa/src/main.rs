extern crate mimalloc;
use mimalloc::MiMalloc;

#[global_allocator]
static GLOBAL: MiMalloc = MiMalloc;

extern crate clap;
extern crate indicatif;
extern crate rand;
extern crate rand_chacha;

use std::fs;
use std::fs::{File, metadata};
use std::io::{BufReader, BufWriter, Seek, SeekFrom, Write, Read};
use std::path::Path;

use rand::prelude::*;
use rand_chacha::ChaCha20Rng;

use clap::{load_yaml, App, ArgMatches};
use indicatif::{HumanBytes, ProgressBar, ProgressIterator, ProgressStyle};


use libsfasta::prelude::*;

fn style_pb(pb: ProgressBar) -> ProgressBar {
    let style = ProgressStyle::default_bar()
        .template("[{spinner:.green}] {bar:30.green/yellow} {bytes:.cyan}/{total_bytes:.blue} ({eta})")
        .progress_chars("█▇▆▅▄▃▂▁  ")
        .tick_chars("ACTGN🧬");
    pb.set_style(style);
    pb
}

fn main() {
    let yaml = load_yaml!("cli.yaml");
    let matches = App::from(yaml).get_matches();

    if let Some(matches) = matches.subcommand_matches("convert") {
        convert(&matches);
    }

    // TODO: Make this faster but putting the decompression into another thread...
    // Probs not worth it (NT takes ~10m on my machine)
    if let Some(matches) = matches.subcommand_matches("summarize") {
        let fasta_filename = matches.value_of("input").unwrap();
        let metadata = fs::metadata(fasta_filename).expect("Unable to get filesize");
        let pb = ProgressBar::new(metadata.len());
        let pb = style_pb(pb);

        let buf = generic_open_file_pb(pb, fasta_filename);
        // let buf = pb.wrap_read(buf.2);
        let summary = summarize_fasta(&mut BufReader::with_capacity(4 * 1024 * 1024, buf.2));
        println!("File: {}", fasta_filename);
        println!("Total Entries: {}", summary.0);
        println!("Total Sequence: {}", summary.2.iter().sum::<usize>());
    }

    /*
    if let Some(matches) = matches.subcommand_matches("stats") {
        stats(&matches);
    }
    if let Some(matches) = matches.subcommand_matches("split") {
        split(&matches);
    }
    if let Some(matches) = matches.subcommand_matches("index") {
        index_file(&matches);
    } */
}

fn convert(matches: &ArgMatches) {
    let fasta_filename = matches.value_of("input").unwrap();

    let metadata = fs::metadata(fasta_filename).expect("Unable to get filesize");
    let pb = ProgressBar::new(metadata.len());
    let pb = style_pb(pb);

    let buf = generic_open_file_pb(pb, fasta_filename);
    // let buf = pb.wrap_read(buf.2);
    let summary = summarize_fasta(&mut BufReader::with_capacity(16 * 1024 * 1024, buf.2));
    println!("File: {}", fasta_filename);
    println!("Total Entries: {}", summary.0);

    let path = Path::new(fasta_filename);
    let output_name = path.clone().with_extension(".sfasta");
    let mut output = match File::create(output_name) {
        Err(why) => panic!("couldn't create: {}", why),
        Ok(file) => file,
    };
    // let mut output = BufWriter::with_capacity(64 * 1024 * 1024, output);

    let metadata = fs::metadata(fasta_filename).expect("Unable to get filesize");
    let pb = ProgressBar::new(metadata.len());
    let pb = style_pb(pb);

    let buf = generic_open_file_pb(pb, fasta_filename);
    let buf = BufReader::with_capacity(4 * 1024 * 1024, buf.2);

    convert_fasta(buf, &mut output, 4 * 1024 * 1024, 32);
}

pub fn generic_open_file_pb(pb: ProgressBar, filename: &str) -> (usize, bool, Box<dyn Read + Send>) {
    let filesize = metadata(filename)
        .unwrap_or_else(|_| panic!("{}", &format!("Unable to open file: {}", filename)))
        .len();

    let file = match File::open(filename) {
        Err(why) => panic!("Couldn't open {}: {}", filename, why.to_string()),
        Ok(file) => file,
    };

    let file = BufReader::with_capacity(8 * 1024 * 1024, file);
    let mut compressed: bool = false;
    let file = pb.wrap_read(file);

    let fasta: Box<dyn Read + Send> = if filename.ends_with("gz") {
        compressed = true;
        Box::new(flate2::read::GzDecoder::new(file))
    } else if filename.ends_with("snappy") || filename.ends_with("sz") || filename.ends_with("sfai")
    {
        compressed = true;
        Box::new(snap::read::FrameDecoder::new(file))
    } else {
        Box::new(file)
    };

    (filesize as usize, compressed, fasta)
}