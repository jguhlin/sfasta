use libsfasta as sfasta;
use libsfasta::create_index;

/*
extern crate mimalloc;
use mimalloc::MiMalloc;

#[global_allocator]
static GLOBAL: MiMalloc = MiMalloc; */

extern crate indicatif;
extern crate rand;
extern crate rand_chacha;

use rand::prelude::*;
use rand_chacha::ChaCha20Rng;
use std::fs::File;
use std::io::{BufWriter, Seek, SeekFrom, Write};
use std::path::Path;

use indicatif::{HumanBytes, ProgressBar, ProgressIterator, ProgressStyle};

extern crate clap;
use clap::{load_yaml, App, ArgMatches};

fn main() {
    let yaml = load_yaml!("cli.yaml");
    let matches = App::from(yaml).get_matches();

    if let Some(matches) = matches.subcommand_matches("convert") {
        convert(&matches);
    }
    if let Some(matches) = matches.subcommand_matches("stats") {
        stats(&matches);
    }
    if let Some(matches) = matches.subcommand_matches("split") {
        split(&matches);
    }
    if let Some(matches) = matches.subcommand_matches("index") {
        index(&matches);
    }
}

fn convert(matches: &ArgMatches) {
    let fasta_filename = matches.value_of("input").unwrap();
    let path = Path::new(fasta_filename);
    sfasta::convert_fasta_file(
        &fasta_filename,
        &path.file_stem().unwrap().to_str().unwrap(),
    );
}

// TODO: This should be in sfasta
fn get_pos(fh: &mut dyn Seek) -> u64 {
    let pos = fh
        .seek(SeekFrom::Current(0))
        .expect("Unable to work with seek API");
    pos
}

// TODO: This should be in sfasta
fn write_entry_to_file(
    mut fh: &mut Box<dyn WriteAndSeek>,
    ids: &mut Vec<String>,
    locations: &mut Vec<u64>,
    block_ids: &mut Vec<(String, u64)>,
    block_locations: &mut Vec<u64>,
    s: (
        sfasta::EntryCompressedHeader,
        Vec<sfasta::EntryCompressedBlock>,
    ),
) {
    let mut pos = get_pos(fh);
    ids.push(s.0.id.clone());
    locations.push(pos);

    bincode::serialize_into(&mut fh, &s.0).expect("Unable to write to bincode output");

    for block in s.1 {
        pos = get_pos(fh);
        block_ids.push((block.id.clone(), block.block_id));
        block_locations.push(pos);

        bincode::serialize_into(&mut fh, &block).expect("Unable to encode block to training file");
    }
}

// TODO: This should be in sfasta
trait WriteAndSeek: Write + Seek + Send {}
impl<T: Write + Seek + Send> WriteAndSeek for T {}

fn write_to_file<I>(
    fh: &mut Box<dyn WriteAndSeek>,
    seqs: I,
) -> (Vec<String>, Vec<u64>, Vec<(String, u64)>, Vec<u64>)
where
    I: Iterator<
        Item = (
            sfasta::EntryCompressedHeader,
            Vec<sfasta::EntryCompressedBlock>,
        ),
    >,
{
    let starting_size = 1024;
    let mut ids = Vec::with_capacity(starting_size);
    let mut locations = Vec::with_capacity(starting_size);
    let mut block_ids = Vec::with_capacity(starting_size * 1024);
    let mut block_locations: Vec<u64> = Vec::with_capacity(starting_size * 1024);

    seqs.for_each(|s| {
        write_entry_to_file(
            fh,
            &mut ids,
            &mut locations,
            &mut block_ids,
            &mut block_locations,
            s,
        );
    });

    (ids, locations, block_ids, block_locations)
}

fn split(matches: &ArgMatches) {
    // TODO: Make the index as we go along, rather than at the end!
    let sfasta_filename = matches.value_of("input").unwrap();
    let output_prefix = matches.value_of("output").unwrap();
    let training_split = matches.value_of_t::<f32>("training").unwrap();
    let length_mode = matches.is_present("length_mode");
    let seed = matches.value_of_t::<u64>("seed").unwrap();

    let mut rng = ChaCha20Rng::seed_from_u64(seed);

    let train_filename = output_prefix.to_string() + "_train.sfasta";
    let valid_filename = output_prefix.to_string() + "_validation.sfasta";

    let out_train = File::create(&train_filename).expect("Unable to write to file");
    let mut out_train: Box<dyn WriteAndSeek> = Box::new(BufWriter::new(out_train));

    let out_valid = File::create(&valid_filename).expect("Unable to write to file");
    let mut out_valid: Box<dyn WriteAndSeek> = Box::new(BufWriter::new(out_valid));

    let mut seqs = sfasta::Sequences::new(&sfasta_filename);
    if !length_mode {
        seqs.set_mode(sfasta::SeqMode::Random);
    }
    let seqs = seqs.into_compressed_sequences();

    let mut total_len = 0;
    let mut training = 0;
    let mut validation = 0;

    // Both get a copy of the header
    bincode::serialize_into(&mut out_train, &seqs.header)
        .expect("Unable to write to bincode output");
    bincode::serialize_into(&mut out_valid, &seqs.header)
        .expect("Unable to write to bincode output");

    // Split (based on number of sequences, or on total seq length)

    assert!(seqs.idx.is_some());
    let n = seqs.idx.as_ref().unwrap().0.len();

    let pb = ProgressBar::new(n as u64);

    pb.set_style(
        ProgressStyle::default_bar()
            .template("{spinner:.green} [{bar:40.cyan/blue}] {eta_precise} {msg}")
            .progress_chars("█▛▌▖  ")
            .tick_chars("ACTGN"),
    );

    let mut seqs = seqs.progress_with(pb.clone());

    if !length_mode {
        // Can't do this without an index!
        let npct = (n as f32 * training_split) as usize;
        let idx = write_to_file(&mut out_train, seqs.by_ref().take(npct));
        drop(out_train);
        create_index(&train_filename, idx.0, idx.1, idx.2, idx.3);

        // We work on the remainder, so that none are dropped.
        let idx = write_to_file(&mut out_valid, Box::new(seqs));
        create_index(&valid_filename, idx.0, idx.1, idx.2, idx.3);

        // TODO: I don't think this is complete / working...
        println!("Training: {}\tValidation: {}", npct, n - npct);
    } else {
        let starting_size = 1024;

        let mut ids_train = Vec::with_capacity(starting_size);
        let mut locations_train = Vec::with_capacity(starting_size);
        let mut block_ids_train = Vec::with_capacity(starting_size * 1024);
        let mut block_locations_train: Vec<u64> = Vec::with_capacity(starting_size * 1024);

        let mut ids_valid = Vec::with_capacity(starting_size);
        let mut locations_valid = Vec::with_capacity(starting_size);
        let mut block_ids_valid = Vec::with_capacity(starting_size * 1024);
        let mut block_locations_valid: Vec<u64> = Vec::with_capacity(starting_size * 1024);

        for entry in seqs {
            total_len += entry.0.len;
            let training_goal = (total_len as f32 * training_split) as usize;
            // validation_goal = (total_len as f32 * (1.0 - training_split)) as usize;
            let tchance = (training_goal as f32 - training as f32) / entry.0.len as f32;
            if rng.gen::<f32>() < tchance {
                training += entry.0.len;
                write_entry_to_file(
                    &mut out_train,
                    &mut ids_train,
                    &mut locations_train,
                    &mut block_ids_train,
                    &mut block_locations_train,
                    entry,
                );
            } else {
                validation += entry.0.len;
                write_entry_to_file(
                    &mut out_valid,
                    &mut ids_valid,
                    &mut locations_valid,
                    &mut block_ids_valid,
                    &mut block_locations_valid,
                    entry,
                );
            }

            pb.set_message(&format!(
                "Training Len: {} Validation Len: {}",
                HumanBytes(training),
                HumanBytes(validation)
            ));
        }

        // Drop (to close the files properly)
        drop(out_train);
        drop(out_valid);
        create_index(
            &train_filename,
            ids_train,
            locations_train,
            block_ids_train,
            block_locations_train,
        );
        create_index(
            &valid_filename,
            ids_valid,
            locations_valid,
            block_ids_valid,
            block_locations_valid,
        );

        println!(
            "Training Seq Length: {}\tValidation Seq Length:{}",
            training, validation
        );
    }
}

fn stats(matches: &ArgMatches) {
    let sfasta_filename = matches.value_of("input").unwrap();
    let (_, idx) = sfasta::open_file(&sfasta_filename);
    let mut seqs = sfasta::Sequences::new(&sfasta_filename);
    seqs.set_mode(sfasta::SeqMode::Random);
    let seqs = seqs.into_compressed_sequences();

    println!("Index Available: {}", idx.is_some());
    println!("Index Length: {}", idx.unwrap().0.len());
    println!("Header ID: {}", seqs.header.id.as_ref().unwrap());
    println!("Header Comment: {:#?}", seqs.header.comment);
    println!("Header Citation: {:#?}", seqs.header.citation);

    let ids: Vec<(String, u64)> = seqs.take(5).map(|seq| (seq.0.id, seq.0.len)).collect();
    println!("\nRandom (up to 5) IDs");
    for e in ids {
        println!("id: {} Length: {}", e.0, e.1);
    }

    let mut seqs = sfasta::Sequences::new(&sfasta_filename);
    seqs.set_mode(sfasta::SeqMode::Random);
    let mut seqs = seqs.into_compressed_sequences();
    let ec = seqs.next().unwrap();

    /*let seq = ec.clone().decompress();
    let len = if seq.len < 500 { seq.len as usize } else { 500 };
    println!("{}", std::str::from_utf8(&seq.seq[0..len]).unwrap());
    println!("{} {}", seq.len, ec.compressed_seq.len());*/
}

fn index(matches: &ArgMatches) {
    let sfasta_filename = matches.value_of("input").unwrap();
    sfasta::index(sfasta_filename);
}
