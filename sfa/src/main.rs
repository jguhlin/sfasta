// // When not windows, use mimalloc
// #[cfg(not(windows))]
// #[global_allocator]
// #[cfg(not(windows))]
// static GLOBAL: MiMalloc = MiMalloc;

// static MEM: &str = "Mimalloc";

// use mimalloc::MiMalloc;
//
// #[global_allocator]
// static GLOBAL: MiMalloc = MiMalloc;

extern crate clap;
extern crate indicatif;
extern crate rand;
extern crate rand_chacha;

use std::{
    fs::{self, File},
    io::{BufReader, Read, Write},
    path::Path
};

#[cfg(unix)]
use std::os::fd::AsRawFd;

use clap::{Parser, Subcommand};
use indicatif::{ProgressBar, ProgressStyle};

use libsfasta::prelude::*;

// const GIT_VERSION: &str = git_version!();

fn style_pb(pb: ProgressBar) -> ProgressBar
{
    let style = ProgressStyle::default_bar()
        .template("[{spinner:.green}] 🧬 {bar:25.green/yellow} {bytes:.cyan}/{total_bytes:.blue} ({eta})")
        .unwrap()
        .progress_chars("█▇▆▅▄▃▂▁  ")
        .tick_chars("ACTGN");
    pb.set_style(style);
    pb
}

// TODO: Print out GIT_VERSION in the version text with CLAP (as well
// as the Cargo.toml version).

#[derive(Parser)]
#[clap(arg_required_else_help = true)]
#[clap(name = "sfasta")]
#[clap(author = "Joseph Guhlin <joseph.guhlin@gmail.com>")]
#[clap(about = "Sequence Storage optimized for fast random access", long_about = None)]
#[clap(version = clap::crate_version!())]
struct Cli
{
    #[clap(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands
{
    /// Adds files to myapp
    View
    {
        input: String
    },
    List
    {
        input: String
    },
    Faidx
    {
        input: String, ids: Vec<String>
    },
    Convert
    {
        input: String,

        /// Number of compression threads (Total usage will be this + 2, typically)
        #[clap(short, long)]
        #[clap(default_value_t = 4)]
        threads: u8,

        /// Read metadata from a file (YAML format)
        #[clap(long)]
        metadata: Option<String>,

        #[clap(short, long)]
        noindex: bool,
        
        /// Fast compression profile
        #[clap(long)]
        fast: bool,
        /// Fastest compression profile
        #[clap(long)]
        fastest: bool,
        /// Small compression profile
        #[clap(long)]
        small: bool,
        /// Smallest compression profile
        #[clap(long)]
        smallest: bool,

        /// Use a custom profile (in YAML format)
        #[clap(short, long)]
        profile: Option<String>,

        /// Block size for sequence blocks in kb
        /// 512 (512kbp) is the default
        #[clap(short, long)]
        blocksize: Option<u32>,

        /// Zstandard compression for all. Prefer to use a profile
        #[clap(long)]
        zstd: bool,

        /// LZ4 compression for all
        #[clap(long)]
        lz4: bool,

        /// XZ compression for all
        #[clap(long)]
        xz: bool,

        /// Brotli compression for all
        #[clap(long)]
        brotli: bool,

        /// Snappy compression for all
        #[clap(long)]
        snappy: bool,

        /// GZIP compression for all
        #[clap(long)]
        gzip: bool,

        /// BZIP2 compression for all
        #[clap(long)]
        bzip2: bool,

        /// Disable compression
        #[clap(long)]
        nocompression: bool,
       
        /// Compression level for all
        #[clap(short, long)]
        level: Option<i8>,

        /// Create a dictionary for the block stores?
        #[clap(long)]
        dict: bool,

        /// Number of sample blocks to take for dictionary training
        #[clap(long)]
        #[clap(default_value_t = 100)]
        dict_samples: u64,

        /// Dict size in kb
        #[clap(long)]
        #[clap(default_value_t = 110)]
        dict_size: u64,
    },
    Summarize
    {
        input: String
    },
    Stats
    {
        input: String
    },
    Bp
    {
        input: String
    },
    Index
    {
        input: String
    },
    Split
    {
        input: String,
        output: String,
        training: f32,
        seed: usize,
        #[clap(short, long)]
        length_mode: bool,
    },
}

fn main()
{
    sigpipe::reset();
    env_logger::init();

    let cli = Cli::parse();

    match cli.command {
        Commands::View { input } => view(&input),
        Commands::List { input } => list(&input),
        Commands::Faidx { input, ids } => faidx(&input, &ids),
        Commands::Convert {
            input,
            threads,
            noindex,
            fast,
            fastest,
            small,
            smallest,
            profile,
            blocksize,
            zstd,
            xz,
            brotli,
            snappy,
            gzip,
            bzip2,
            nocompression,
            level,
            dict,
            dict_samples,
            dict_size,
            metadata,
            lz4,
        } => convert(
            &input,
            threads as usize,
            noindex,
            fast,
            fastest,
            small,
            smallest,
            profile,
            blocksize,
            zstd,
            lz4,
            xz,
            brotli,
            snappy,
            gzip,
            bzip2,
            nocompression,
            level,
            dict,
            dict_samples,
            dict_size,
            metadata,
            
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

    // if let Some(matches) = matches.subcommand_matches("convert") {
    // convert(&matches);
    // }
    //
    // if let Some(matches) = matches.subcommand_matches("stats") {
    // let filename = matches.value_of("input").unwrap();
    // let file = match File::open(filename) {
    // Err(why) => panic!("Couldn't open {}: {}", filename,
    // why.to_string()), Ok(file) => file,
    // };
    //
    // let file = BufReader::with_capacity(8 * 1024 * 1024, file);
    // let sfasta = SfastaParser::open_from_buffer(file);
    // println!("Successfully opened SFASTA");
    // println!("Found: {} entries",
    // sfasta.index.as_ref().unwrap().len()); println!("{:#?}",
    // sfasta.index.as_ref().unwrap().ids); }
    //
    // TODO: Make this faster but putting the decompression into
    // another thread... Probs not worth it (NT takes ~10m on my
    // machine) if let Some(matches) =
    // matches.subcommand_matches("summarize") {
    // let fasta_filename = matches.value_of("input").unwrap();
    // let metadata = fs::metadata(fasta_filename).expect("Unable to
    // get filesize"); let pb = ProgressBar::new(metadata.len());
    // let pb = style_pb(pb);
    //
    // let buf = generic_open_file_pb(pb, fasta_filename);
    // let buf = pb.wrap_read(buf.2);
    // let summary = summarize_fasta(&mut BufReader::with_capacity(4 *
    // 1024 * 1024, buf.2)); println!("File: {}", fasta_filename);
    // println!("Total Entries: {}", summary.0);
    // println!("Total Sequence: {}",
    // summary.2.iter().sum::<usize>()); }
    //
    // if let Some(matches) = matches.subcommand_matches("faidx") {
    // let sfasta_filename = matches.value_of("input").unwrap();
    //
    // let in_buf = File::open(sfasta_filename).expect("Unable to open
    // file"); let sfasta =
    // SfastaParser::open_from_buffer(BufReader::with_capacity(4 *
    // 1024 * 1024, in_buf));
    //
    // let ids = matches.values_of("ids").unwrap();
    // for i in ids {
    // let results = sfasta
    // .find(i)
    // .expect(&format!("Unable to find {} in file {}", i,
    // sfasta_filename)) .unwrap();
    // for result in results {
    // let sequence = sfasta
    // .get_sequence(&result.3)
    // .expect("Unable to fetch sequence");
    // println!(">{}", i);
    // println!("{}", from_utf8(&sequence).unwrap());
    // }
    // }
    // }
    //
    // TODO: Store sequence IDs in insertion order at the end of the
    // file insertion-order output should be a custom function
    // (Decompress entire block, output sequences appropriately)
    // So don't need this more random (and slower) method anymore...
    // if let Some(matches) = matches.subcommand_matches("view") {
    // TODO: Add ability to store sequences in order, to reorder
    // sequences, and to identify when sequences are in order and
    // display them that way...
    //
    // let sfasta_filename = matches.value_of("input").unwrap();
    //
    // let in_buf = File::open(sfasta_filename).expect("Unable to open
    // file"); let mut sfasta =
    // SfastaParser::open_from_buffer(BufReader::with_capacity(8 *
    // 1024 * 1024, in_buf));
    //
    // TODO: We could process this in blocks to be more memory
    // efficient... sfasta.decompress_all_ids();
    //
    // let mut bump = Bump::new();
    //
    // for seqid in
    // sfasta.index.as_ref().unwrap().ids.as_ref().unwrap() {
    // let results = sfasta
    // .find(&seqid)
    // .expect(&format!("Unable to find {} in file {}, even though it
    // is in the index! File is likely corrupt, or this
    // is a serious bug.", &seqid, sfasta_filename)) .unwrap();
    //
    // TODO: Disabled bumpalo... for now...
    // for result in results {
    // let sequence = sfasta
    // .get_sequence(&result.3)
    // .expect("Unable to fetch sequence");
    // println!(">{}", seqid);
    // println!("{}", from_utf8(&sequence).unwrap());
    // }
    //
    // bump.reset();
    // }
    // }
    //
    // if let Some(matches) = matches.subcommand_matches("list") {
    // let sfasta_filename = matches.value_of("input").unwrap();
    //
    // let in_buf = File::open(sfasta_filename).expect("Unable to open
    // file"); let mut sfasta =
    // SfastaParser::open_from_buffer(BufReader::with_capacity(32 *
    // 1024 * 1024, in_buf));
    //
    // sfasta.decompress_all_ids();
    //
    // for i in sfasta.index.as_ref().unwrap().ids.as_ref().unwrap().
    // iter() { println!("{}", i);
    // }
    // }

    // if let Some(matches) = matches.subcommand_matches("stats") {
    // stats(&matches);
    // }
    // if let Some(matches) = matches.subcommand_matches("split") {
    // split(&matches);
    // }
    // if let Some(matches) = matches.subcommand_matches("index") {
    // index_file(&matches);
    // }
}

#[inline]
fn print_sequence(
    stdout: &mut std::io::StdoutLock,
    seq: &[u8],
    line_length: usize,
)
{
    let iter = seq.chunks_exact(line_length);
    let seq = iter.remainder();
    iter.for_each(|x| {
        stdout.write_all(x).expect("Unable to write to stdout");
        stdout.write_all(b"\n").expect("Unable to write to stdout");
    });

    stdout.write_all(seq).expect("Unable to write to stdout");
    stdout.write_all(b"\n").expect("Unable to write to stdout");
}

// TODO: Subsequence support
fn faidx(input: &str, ids: &Vec<String>)
{
    let sfasta_filename = input;

    let in_buf = File::open(sfasta_filename).expect("Unable to open file");

    #[cfg(unix)]
    nix::fcntl::posix_fadvise(
        in_buf.as_raw_fd(),
        0,
        0,
        nix::fcntl::PosixFadviseAdvice::POSIX_FADV_RANDOM,
    )
    .expect("Fadvise Failed");

    let in_buf = BufReader::new(in_buf);

    let mut sfasta = SfastaParser::open_from_buffer(in_buf, false).unwrap();

    let stdout = std::io::stdout();
    let mut stdout = stdout.lock();

    for i in ids {
        let result = match sfasta.find(i) {
            Ok(Some(x)) => x,
            Ok(None) => panic!("No SeqLoc found"),
            Err(_) => panic!("Unable to fetch seqloc"),
        };

        let headers = result.get_headers();
        let header = if result.has_headers() {
            format!(
                " {}",
                sfasta.get_header(&headers).expect("Unable to fetch header")
            )
        } else {
            "".to_string()
        };

        write!(stdout, ">{}{}\n", i, header)
            .expect("Unable to write to stdout");

        let sequence = sfasta
            .get_sequence(result.get_sequence(), result.get_masking())
            .expect("Unable to fetch sequence");

        print_sequence(&mut stdout, &sequence, 60);
        stdout.flush();
    }
}

// TODO: Line length as an argument
fn view(input: &str)
{
    let sfasta_filename = input;

    let in_buf = File::open(sfasta_filename).expect("Unable to open file");

    #[cfg(unix)]
    nix::fcntl::posix_fadvise(
        in_buf.as_raw_fd(),
        0,
        0,
        nix::fcntl::PosixFadviseAdvice::POSIX_FADV_RANDOM,
    )
    .expect("Fadvise Failed");

    let mut sfasta =
        SfastaParser::open_from_buffer(BufReader::new(in_buf), true).unwrap();

    if sfasta.seqlocs.is_none() {
        panic!("File is empty or corrupt");
    }

    let line_length = 80;

    let stdout = std::io::stdout().lock();
    let mut stdout = std::io::BufWriter::new(stdout);
    let common = b"> \n";

    // let seqlocs = sfasta.get_seqlocs().unwrap().unwrap().to_vec();

    // for seqloc in seqlocs {
    // let id = sfasta.get_id(seqloc.get_ids()).unwrap();
    //
    // stdout.write_all(&common[..1]).unwrap();
    // stdout.write_all(id.as_bytes()).unwrap();
    //
    // if seqloc.has_headers() {
    // stdout
    // .write_all(
    // sfasta
    // .get_header(seqloc.get_headers())
    // .expect("Unable to fetch header")
    // .as_bytes(),
    // )
    // .unwrap();
    // }
    //
    // stdout.write_all(b"\n").unwrap();
    //
    // let sequence = sfasta
    // .get_sequence(seqloc.get_sequence(), seqloc.get_masking())
    // .expect("Unable to fetch sequence");
    //
    // #[cfg(nightly)]
    // {
    // let newlines = (0..1).map(|_|
    // std::io::IoSlice::new(b"\n")).cycle(); let x = sequence
    // .chunks(line_length)
    // .map(|x| std::io::IoSlice::new(x))
    // .zip(newlines)
    // .map(|x| [x.0, x.1])
    // .flatten()
    // .collect::<Vec<_>>();
    // stdout.write_all_vectored(&mut x).unwrap();
    // }
    //
    // #[cfg(not(nightly))]
    // {
    // sequence.chunks(line_length).for_each(|x| {
    // stdout.write_all(x).unwrap();
    // stdout.write_all(b"\n").unwrap();
    // });
    // }
    //
    // 60 matches samtools faidx output
    // But 80 is common elsewhere...
    //
    // print_sequence(&mut stdout, &sequence, 80);
    // stdout.flush().expect("Unable to flush stdout buffer");
    //
    // }
}

fn list(input: &str)
{
    let sfasta_filename = input;

    let in_buf = File::open(sfasta_filename).expect("Unable to open file");

    #[cfg(unix)]
    nix::fcntl::posix_fadvise(
        in_buf.as_raw_fd(),
        0,
        0,
        nix::fcntl::PosixFadviseAdvice::POSIX_FADV_RANDOM,
    )
    .expect("Fadvise Failed");

    let in_buf = BufReader::new(in_buf);
    let mut sfasta = SfastaParser::open_from_buffer(in_buf, false).unwrap();

    if sfasta.seqlocs.is_none() {
        panic!("File is empty of corrupt");
    }

    for i in 0..sfasta.len() {
        let seqloc = match sfasta.get_seqloc(i) {
            Ok(Some(x)) => x,
            Ok(None) => panic!("No SeqLoc found"),
            Err(_) => panic!("Unable to fetch seqloc"),
        }
        .clone();
        let id = &sfasta.get_id(seqloc.get_ids()).unwrap();
        println!("{}", id);
    }
}

// TODO: Set metadata
// TODO: Set masking option
// TODO: Block sizes, index compression type, etc...
fn convert(
    fasta_filename: &str,
    threads: usize,
    noindex: bool,
    fast: bool,
    fastest: bool,
    small: bool,
    smallest: bool,
    profile: Option<String>,
    blocksize: Option<u32>,
    zstd: bool,
    lz4: bool,
    xz: bool,
    brotli: bool,
    snappy: bool,
    gzip: bool,
    bzip2: bool,
    nocompression: bool,
    level: Option<i8>,
    dict: bool,
    dict_samples: u64,
    dict_size: u64,
    metadata: Option<String>,
)
{
    let fs_metadata =
        fs::metadata(fasta_filename).expect("Unable to get filesize");
    let pb = ProgressBar::new(fs_metadata.len());
    let pb = style_pb(pb);

    let path = Path::new(fasta_filename);
    let output_name = path.with_extension("sfasta");
    let output = match File::create(output_name) {
        Err(why) => panic!("couldn't create: {}", why),
        Ok(file) => file,
    };

    let buf = generic_open_file_pb(pb, fasta_filename);
    let buf = buf.1;

    let mut converter = Converter::default();
    converter.with_threads(threads);
    log::info!("Using {} threads", threads);

    if fast {
        log::info!("Using fast compression profile");
        let profile = CompressionProfile::fast();
        converter.with_compression_profile(profile);
    }
    if fastest {
        log::info!("Using fastest compression profile");
        let profile = CompressionProfile::fastest();
        converter.with_compression_profile(profile);
    }
    if small {
        log::info!("Using small compression profile");
        let profile = CompressionProfile::small();
        converter.with_compression_profile(profile);
    }
    if smallest {
        log::info!("Using smallest compression profile");
        let profile = CompressionProfile::smallest();
        converter.with_compression_profile(profile);
    }

    if profile.is_some() {
        let profile = profile.as_ref().unwrap();
        let profile = std::fs::read_to_string(profile)
            .expect("Unable to read profile file");
        let profile: CompressionProfile =
            serde_yml::from_str(&profile).expect("Unable to parse profile");
        converter.with_compression_profile(profile);
    }

    if metadata.is_some() {
        let metadata = metadata.as_ref().unwrap();
        let metadata = std::fs::read_to_string(metadata)
            .expect("Unable to read metadata file");
        let metadata: Metadata =
            serde_yml::from_str(&metadata).expect("Unable to parse metadata");
        converter.with_metadata(metadata);
    }

    // TODO: Handle all of the compression options...
    // TODO: Warn if more than one compression option specified
    let mut compression_type = CompressionType::default();

    let mut compression_set = false;
    if zstd {
        compression_type = CompressionType::ZSTD;
        compression_set = true;
    } 
    if xz {
        compression_type = CompressionType::XZ;
        if compression_set {
            log::warn!("Multiple compression types specified -- Using XZ");
        }
        compression_set = true;
    }
    if brotli {
        compression_type = CompressionType::BROTLI;
        if compression_set {
            log::warn!("Multiple compression types specified -- Using Brotli");
        }
        compression_set = true;
    } 
    if gzip {
        println!("🤨");
        compression_type = CompressionType::GZIP;
        if compression_set {
            log::warn!("Multiple compression types specified -- Using Gzip");
        }
        compression_set = true;
    } 
    if snappy {
        compression_type = CompressionType::SNAPPY;
        if compression_set {
            log::warn!("Multiple compression types specified -- Using Snappy");
        }
        compression_set = true;
    } 
    if nocompression {
        compression_type = CompressionType::NONE;
        if compression_set {
            log::warn!("Multiple compression types specified -- Using None");
        }
        compression_set = true;
    } 
    if bzip2 {
        compression_type = CompressionType::BZIP2;
        if compression_set {
            log::warn!("Multiple compression types specified -- Using Bzip2");
        }
        compression_set = true;
    }
    if lz4 {
        compression_type = CompressionType::LZ4;
        if compression_set {
            log::warn!("Multiple compression types specified -- Using LZ4");
        }
        compression_set = true;
    }

    if compression_set {
        let level = match level {
            Some(x) => x,
            None => compression_type.default_compression_level(),
        };

        converter
            .with_compression(compression_type, level);

        // If any profiles were set, issue a warning that they were overridden
        if fast || fastest || small || smallest || profile.is_some() {
            log::warn!("Compression profile overridden by manual compression settings");
        }
    }

    if let Some(size) = blocksize {
        if size as usize * 1024 > u32::MAX as usize {
            panic!("Block size too large");
        }
        converter.with_block_size(size as usize * 1024);
    }

    if dict {
        converter.with_dict(dict_samples, dict_size * 1024);
    }

    if noindex {
        log::info!("Building without an ID index");
        converter.without_index();
    }

    let mut in_buf = BufReader::new(buf);

    let out_fh = Box::new(std::io::BufWriter::new(output));

    let _out_fh = converter.convert(&mut in_buf, out_fh);
}

pub fn generic_open_file_pb(
    pb: ProgressBar,
    filename: &str,
) -> (usize, indicatif::ProgressBarIter<File>)
{
    let filesize = fs::metadata(filename)
        .unwrap_or_else(|_| {
            panic!("{}", &format!("Unable to open file: {}", filename))
        })
        .len();

    let file = match File::open(filename) {
        Err(why) => panic!("Couldn't open {}: {}", filename, why),
        Ok(file) => file,
    };

    #[cfg(unix)]
    nix::fcntl::posix_fadvise(
        file.as_raw_fd(),
        0,
        0,
        nix::fcntl::PosixFadviseAdvice::POSIX_FADV_SEQUENTIAL,
    )
    .expect("Fadvise Failed");

    // let mut compressed: bool = false;
    let file = pb.wrap_read(file);

    // let fasta: Box<dyn Read + Send> = if filename.ends_with("gz") {
    // compressed = true;
    // Box::new(flate2::read::MultiGzDecoder::new(file))
    // } else if filename.ends_with("snappy") || filename.ends_with("sz")
    // || filename.ends_with("sfai") { compressed = true;
    // Box::new(snap::read::FrameDecoder::new(file))
    // } else {
    // Box::new(file)
    // };

    (filesize as usize, file)
}

pub fn generic_open_file(filename: &str)
    -> (usize, bool, Box<dyn Read + Send>)
{
    let filesize = fs::metadata(filename)
        .unwrap_or_else(|_| {
            panic!("{}", &format!("Unable to open file: {}", filename))
        })
        .len();

    let file = match File::open(filename) {
        Err(why) => panic!("Couldn't open {}: {}", filename, why),
        Ok(file) => file,
    };

    let mut compressed: bool = false;

    let fasta: Box<dyn Read + Send> = if filename.ends_with("gz") {
        compressed = true;
        Box::new(flate2::read::MultiGzDecoder::new(file))
    } else if filename.ends_with("snappy")
        || filename.ends_with("sz")
        || filename.ends_with("sfai")
    {
        compressed = true;
        Box::new(snap::read::FrameDecoder::new(file))
    } else {
        Box::new(file)
    };

    (filesize as usize, compressed, fasta)
}
