extern crate mimalloc;
use mimalloc::MiMalloc;

#[global_allocator]
static GLOBAL: MiMalloc = MiMalloc;

extern crate indicatif;
extern crate rand;
extern crate rand_chacha;
extern crate clap;

use std::fs::File;
use std::io::{BufWriter, Seek, SeekFrom, Write};
use std::path::Path;

use rand::prelude::*;
use rand_chacha::ChaCha20Rng;

use indicatif::{HumanBytes, ProgressBar, ProgressIterator, ProgressStyle};
use clap::{load_yaml, App, ArgMatches};

use libsfasta::prelude::*;

fn main() {
    let yaml = load_yaml!("cli.yaml");
    let matches = App::from(yaml).get_matches();

    if let Some(matches) = matches.subcommand_matches("convert") {
        convert(&matches);
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
    let path = Path::new(fasta_filename);
    let mut output = match File::create("converted.sfa") {
        Err(why) => panic!("couldn't create: {}", why),
        Ok(file) => file,
    };
    // let mut output = BufWriter::with_capacity(64 * 1024 * 1024, output);
    convert_fasta(
        &fasta_filename,
        &mut output,
        8 * 1024 * 1024,
        32,
    );
}
