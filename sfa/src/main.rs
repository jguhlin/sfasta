extern crate mimalloc;
use mimalloc::MiMalloc;

#[global_allocator]
static GLOBAL: MiMalloc = MiMalloc;

extern crate clap;
extern crate indicatif;
extern crate rand;
extern crate rand_chacha;

use git_version::git_version;

use std::fs;
use std::fs::{metadata, File};
use std::io::{BufReader, Read};
use std::path::Path;

use clap::{arg, command, Command, Parser, Subcommand};
use indicatif::{ProgressBar, ProgressStyle};
use simdutf8::basic::from_utf8;

use libsfasta::prelude::*;

use libsfasta::CompressionType;

// const GIT_VERSION: &str = git_version!();

fn style_pb(pb: ProgressBar) -> ProgressBar {
    let style = ProgressStyle::default_bar()
        .template(
            "[{spinner:.green}] 🧬 {bar:30.green/yellow} {bytes:.cyan}/{total_bytes:.blue} ({eta})",
        )
        .unwrap()
        .progress_chars("█▇▆▅▄▃▂▁  ")
        .tick_chars("ACTGN");
    pb.set_style(style);
    pb
}

// TODO: Print out GIT_VERSION in the version text with CLAP (as well as the Cargo.toml version).

#[derive(Parser)]
#[clap(arg_required_else_help = true)]
#[clap(name = "sfasta")]
#[clap(author = "Joseph Guhlin <joseph.guhlin@gmail.com>")]
#[clap(about = "Sequence Storage optimized for fast random access", long_about = None)]
#[clap(version = clap::crate_version!())]
struct Cli {
    #[clap(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Adds files to myapp
    View {
        input: String,
    },
    List {
        input: String,
    },
    Faidx {
        input: String,
        ids: Vec<String>,
    },
    Convert {
        input: String,
        #[clap(short, long)]
        #[clap(default_value_t = 4)]
        threads: u8,
        #[clap(short, long)]
        noindex: bool,
        #[clap(short, long)]
        zstd: bool,
        #[clap(short, long)]
        lz4: bool,
        #[clap(short, long)]
        xz: bool,
        #[clap(short, long)]
        brotli: bool,
        #[clap(short, long)]
        snappy: bool,
        #[clap(short, long)]
        gzip: bool,
        #[clap(short, long)]
        none: bool,
        /// Block size for sequence blocks / 1024
        /// 4Mb (4096) is the default
        #[clap(short, long)]
        blocksize: Option<u64>,
    },
    Summarize {
        input: String,
    },
    Stats {
        input: String,
    },
    Bp {
        input: String,
    },
    Index {
        input: String,
    },
    Split {
        input: String,
        output: String,
        training: f32,
        seed: usize,
        #[clap(short, long)]
        length_mode: bool,
    },
}

fn main() {
    sigpipe::reset();
    env_logger::init();
    let cli = Cli::parse();

    match &cli.command {
        Commands::View { input } => view(input),
        Commands::List { input } => list(input),
        Commands::Faidx { input, ids } => faidx(&input, &ids),
        Commands::Convert {
            input,
            threads,
            noindex,
            zstd,
            lz4,
            xz,
            brotli,
            snappy,
            gzip,
            none,
            blocksize,
        } => convert(
            &input,
            *threads as usize,
            *zstd,
            *lz4,
            *xz,
            *gzip,
            *brotli,
            *snappy,
            *none,
            *noindex,
            *blocksize,
        ),
        Commands::Summarize { input } => todo!(),
        Commands::Stats { input } => todo!(),
        Commands::Bp { input } => todo!(),
        Commands::Index { input } => todo!(),
        Commands::Split {
            input,
            output,
            training,
            seed,
            length_mode,
        } => todo!(),
    }

    /*
    if let Some(matches) = matches.subcommand_matches("convert") {
        convert(&matches);
    }

    if let Some(matches) = matches.subcommand_matches("stats") {
        let filename = matches.value_of("input").unwrap();
        let file = match File::open(filename) {
            Err(why) => panic!("Couldn't open {}: {}", filename, why.to_string()),
            Ok(file) => file,
        };

        let file = BufReader::with_capacity(8 * 1024 * 1024, file);
        let sfasta = SfastaParser::open_from_buffer(file);
        println!("Successfully opened SFASTA");
        println!("Found: {} entries", sfasta.index.as_ref().unwrap().len());
        println!("{:#?}", sfasta.index.as_ref().unwrap().ids);
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

    if let Some(matches) = matches.subcommand_matches("faidx") {
        let sfasta_filename = matches.value_of("input").unwrap();

        let in_buf = File::open(sfasta_filename).expect("Unable to open file");
        let sfasta =
            SfastaParser::open_from_buffer(BufReader::with_capacity(4 * 1024 * 1024, in_buf));

        let ids = matches.values_of("ids").unwrap();
        for i in ids {
            let results = sfasta
                .find(i)
                .expect(&format!("Unable to find {} in file {}", i, sfasta_filename))
                .unwrap();
            for result in results {
                let sequence = sfasta
                    .get_sequence(&result.3)
                    .expect("Unable to fetch sequence");
                println!(">{}", i);
                println!("{}", from_utf8(&sequence).unwrap());
            }
        }
    }

    // TODO: Store sequence IDs in insertion order at the end of the file
    // insertion-order output should be a custom function
    // (Decompress entire block, output sequences appropriately)
    // So don't need this more random (and slower) method anymore...
    if let Some(matches) = matches.subcommand_matches("view") {
        // TODO: Add ability to store sequences in order, to reorder sequences, and
        // to identify when sequences are in order and display them that way...

        let sfasta_filename = matches.value_of("input").unwrap();

        let in_buf = File::open(sfasta_filename).expect("Unable to open file");
        let mut sfasta =
            SfastaParser::open_from_buffer(BufReader::with_capacity(8 * 1024 * 1024, in_buf));

        // TODO: We could process this in blocks to be more memory efficient...
        sfasta.decompress_all_ids();

        let mut bump = Bump::new();

        for seqid in sfasta.index.as_ref().unwrap().ids.as_ref().unwrap() {
            let results = sfasta
                .find(&seqid)
                .expect(&format!("Unable to find {} in file {}, even though it is in the index! File is likely corrupt, or this is a serious bug.", &seqid, sfasta_filename))
                .unwrap();

            // TODO: Disabled bumpalo... for now...
            for result in results {
                let sequence = sfasta
                    .get_sequence(&result.3)
                    .expect("Unable to fetch sequence");
                println!(">{}", seqid);
                println!("{}", from_utf8(&sequence).unwrap());
            }

            bump.reset();
        }
    }

    if let Some(matches) = matches.subcommand_matches("list") {
        let sfasta_filename = matches.value_of("input").unwrap();

        let in_buf = File::open(sfasta_filename).expect("Unable to open file");
        let mut sfasta =
            SfastaParser::open_from_buffer(BufReader::with_capacity(32 * 1024 * 1024, in_buf));

        sfasta.decompress_all_ids();

        for i in sfasta.index.as_ref().unwrap().ids.as_ref().unwrap().iter() {
            println!("{}", i);
        }
    }
    */

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

fn print_sequence(seq: &[u8], line_length: usize) {
    let mut i = 0;
    while i < seq.len() {
        let end = std::cmp::min(i + line_length, seq.len());
        println!("{}", from_utf8(&seq[i..end]).unwrap());
        i += line_length;
    }
}

// TODO: Subsequence support
fn faidx(input: &str, ids: &Vec<String>) {
    let sfasta_filename = input;

    let in_buf = File::open(sfasta_filename).expect("Unable to open file");

    let mut sfasta = SfastaParser::open_from_buffer(BufReader::with_capacity(512 * 1024, in_buf));

    for i in ids {
        let result = sfasta
            .find(i)
            .expect(&format!("Unable to find {} in file {}", i, sfasta_filename))
            .unwrap();

        if result.headers.is_some() {
            let header = sfasta
                .get_header(&result.headers.as_ref().unwrap())
                .expect("Unable to fetch header");
            println!(">{} {}", i, header);
        } else {
            println!(">{}", i);
        }

        let mut sequence = sfasta
            .get_sequence(&result)
            .expect("Unable to fetch sequence");

        // 60 matches samtools faidx output
        print_sequence(&sequence, 60);
    }
}

fn view(input: &str) {
    let sfasta_filename = input;

    let in_buf = File::open(sfasta_filename).expect("Unable to open file");
    let mut sfasta = SfastaParser::open_from_buffer(BufReader::with_capacity(512 * 1024, in_buf));

    if sfasta.seqlocs.is_none() {
        panic!("File is empty or corrupt");
    }

    for i in 0..sfasta.len() {
        let seqloc = &sfasta.get_seqloc(i);
        let id = &sfasta.get_id(&seqloc.ids.as_ref().unwrap()).unwrap();
        if seqloc.headers.is_some() {
            let header = sfasta
                .get_header(&seqloc.headers.as_ref().unwrap())
                .expect("Unable to fetch header");
            println!(">{} {}", id, header);
        } else {
            println!(">{}", id);
        }

        let sequence = sfasta
            .get_sequence(&seqloc)
            .expect("Unable to fetch sequence");

        // 60 matches samtools faidx output
        print_sequence(&sequence, 80);
    }
}

fn list(input: &str) {
    let sfasta_filename = input;

    let in_buf = File::open(sfasta_filename).expect("Unable to open file");
    let mut sfasta = SfastaParser::open_from_buffer(BufReader::with_capacity(128 * 1024, in_buf));

    if sfasta.seqlocs.is_none() {
        panic!("File is empty of corrupt");
    }

    for i in 0..sfasta.len() {
        let seqloc = &sfasta.get_seqloc(i);
        let id = &sfasta.get_id(seqloc.ids.as_ref().unwrap()).unwrap();
        println!("{}", id);
    }
}

// TODO: Set metadata
// TODO: Set masking option
// TODO: Block sizes, index compression type, etc...
fn convert(
    fasta_filename: &str,
    threads: usize,
    zstd: bool,
    lz4: bool,
    xz: bool,
    gzip: bool,
    brotli: bool,
    snappy: bool,
    none: bool,
    noindex: bool,
    blocksize: Option<u64>,
) {
    let metadata = fs::metadata(fasta_filename).expect("Unable to get filesize");
    let pb = ProgressBar::new(metadata.len());
    let pb = style_pb(pb);

    let path = Path::new(fasta_filename);
    let output_name = path.with_extension("sfasta");
    let output = match File::create(output_name) {
        Err(why) => panic!("couldn't create: {}", why),
        Ok(file) => file,
    };

    let buf = generic_open_file_pb(pb, fasta_filename);
    let mut buf = buf.2;

    let (s, r) = crossbeam::channel::bounded(128);

    let io_thread = std::thread::spawn(move || {
        let mut buffer: [u8; 256 * 1024] = [0; 256 * 1024];
        while let Ok(bytes_read) = buf.read(&mut buffer) {
            if bytes_read == 0 {
                s.send(libsfasta::utils::ReaderData::EOF).unwrap();
                break;
            }
            s.send(libsfasta::utils::ReaderData::Data(buffer[..bytes_read].to_vec())).unwrap();

        }
    });

    // TODO: Handle all of the compression options...
    // TODO: Warn if more than one compression option specified
    let mut compression_type = CompressionType::default();

    if zstd {
        compression_type = CompressionType::ZSTD;
    } else if lz4 {
        compression_type = CompressionType::LZ4;
    } else if xz {
        compression_type = CompressionType::XZ;
    } else if brotli {
        compression_type = CompressionType::BROTLI;
    } else if gzip {
        println!("🤨");
        compression_type = CompressionType::GZIP;
    } else if snappy {
        compression_type = CompressionType::SNAPPY;
    } else if none {
        compression_type = CompressionType::NONE;
    }

    let mut converter = Converter::default()
        .with_threads(threads)
        .with_compression_type(compression_type);

    if let Some(size) = blocksize {
        converter = converter.with_block_size(size as usize * 1024);
    }

    if noindex {
        println!("Noindex received -- But this doesn't work yet -- Here be dragons");
        converter = converter.without_index();
    }

    let buf = libsfasta::utils::CrossbeamReader::from_channel(r);

    converter.convert_fasta(buf, output);
    io_thread.join().expect("Unable to join IO thread");
}

pub fn generic_open_file_pb(
    pb: ProgressBar,
    filename: &str,
) -> (usize, bool, Box<dyn Read + Send>) {
    let filesize = metadata(filename)
        .unwrap_or_else(|_| panic!("{}", &format!("Unable to open file: {}", filename)))
        .len();

    let file = match File::open(filename) {
        Err(why) => panic!("Couldn't open {}: {}", filename, why),
        Ok(file) => file,
    };

    let mut compressed: bool = false;
    let file = pb.wrap_read(file);

    let fasta: Box<dyn Read + Send> = if filename.ends_with("gz") {
        compressed = true;
        Box::new(flate2::read::MultiGzDecoder::new(file))
    } else if filename.ends_with("snappy") || filename.ends_with("sz") || filename.ends_with("sfai")
    {
        compressed = true;
        Box::new(snap::read::FrameDecoder::new(file))
    } else {
        Box::new(file)
    };

    (filesize as usize, compressed, fasta)
}

pub fn generic_open_file(filename: &str) -> (usize, bool, Box<dyn Read + Send>) {
    let filesize = metadata(filename)
        .unwrap_or_else(|_| panic!("{}", &format!("Unable to open file: {}", filename)))
        .len();

    let file = match File::open(filename) {
        Err(why) => panic!("Couldn't open {}: {}", filename, why),
        Ok(file) => file,
    };

    let mut compressed: bool = false;

    let fasta: Box<dyn Read + Send> = if filename.ends_with("gz") {
        compressed = true;
        Box::new(flate2::read::MultiGzDecoder::new(file))
    } else if filename.ends_with("snappy") || filename.ends_with("sz") || filename.ends_with("sfai")
    {
        compressed = true;
        Box::new(snap::read::FrameDecoder::new(file))
    } else {
        Box::new(file)
    };

    (filesize as usize, compressed, fasta)
}
