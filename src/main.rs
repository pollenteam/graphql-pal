mod query_extractor;
mod schema_stats;

use crate::query_extractor::SkippedResult;
use colored::*;
use globwalk::DirEntry;
use globwalk::GlobWalkerBuilder;
use graphql_parser::parse_query;
use indicatif::ProgressBar;
use indicatif::ProgressIterator;
use indicatif::ProgressStyle;
use query_extractor::extract_queries_from_file;
use schema_stats::generate_schema_stats;
use std::convert::TryInto;
use std::ffi::OsStr;
use std::fs::read_to_string;
use std::fs::File;
use std::io::Write;
use std::path::Path;
use structopt::StructOpt;

#[derive(StructOpt, Debug)]
#[structopt(about = "GraphQL pal")]
enum Command {
    ExtractQueries {
        #[structopt()]
        path: String,
        #[structopt(default_value = "queries.graphql")]
        output: String,
        #[structopt(short = "e", help = "Path(s) to exclude")]
        exclude: Vec<String>,
    },
    SchemaStats {
        #[structopt()]
        documents: String,
        #[structopt()]
        schema: String,
        #[structopt(
            long,
            help = "This will include fields from fragments, even if they are not used"
        )]
        include_fragments: bool,
        #[structopt(long, help = "Output results as JSON")]
        json: bool,
    },
}

#[derive(StructOpt, Debug)]
struct Opt {
    #[structopt(subcommand)]
    cmd: Command,
}

fn write_message(message: ColoredString) {
    println!("  {}", message);
}

fn query_is_valid(query: &str) -> bool {
    match parse_query::<&str>(query) {
        Err(_) => {
            return false;
        }
        Ok(_) => true,
    }
}

fn main() {
    let opt = Opt::from_args();

    match opt.cmd {
        Command::ExtractQueries {
            path,
            output,
            exclude,
        } => {
            println!("");
            write_message("## Extracting documents".magenta().bold());
            println!("");
            write_message(String::from(format!("Extracting documents from {}\n", path)).normal());

            let mut paths = vec![
                "*.{js,ts,tsx,graphql}".to_string(),
                "!node_modules".to_string(),
                "!.next".to_string(),
                "!.layers".to_string(),
            ];
            paths.extend(exclude.into_iter().map(|x| format!("!{}", x)));

            let walker = GlobWalkerBuilder::from_patterns(path.clone(), &paths)
                .follow_links(false)
                .build()
                .unwrap();

            let files = walker
                .into_iter()
                .map(|x| x.unwrap())
                .collect::<Vec<DirEntry>>();

            let bar = ProgressBar::new(files.len().try_into().unwrap());

            bar.set_style(
                ProgressStyle::default_bar()
                    .template("  [{elapsed_precise}] {wide_bar:.cyan/blue} {pos:>7}/{len:7} {msg}")
                    .progress_chars("##-"),
            );

            let mut queries: Vec<String> = Vec::new();
            let mut skipped_files: Vec<SkippedResult> = Vec::new();

            for entry in files.iter().progress_with(bar) {
                let path = entry.path();

                if path.extension() == Some(OsStr::new("graphql")) {
                    let q = read_to_string(path);

                    match q {
                        Ok(query) => queries.push(query),
                        Err(e) => skipped_files.push(SkippedResult {
                            path: path.display().to_string(),
                            reason: e.to_string(),
                        }),
                    }
                } else {
                    let q = extract_queries_from_file(path);

                    match q {
                        Some(mut result) => {
                            for query in result.queries {
                                if query_is_valid(&query) {
                                    queries.push(query.to_string());
                                } else {
                                    println!("Invalid query in {}", path.display());
                                    println!("{}", query);
                                    panic!();
                                }
                            }

                            skipped_files.append(&mut result.skipped_files);
                        }
                        None => {}
                    }
                }
            }

            let output_path = Path::new(&output);
            let display = output_path.display();

            let mut file = match File::create(&output_path) {
                Err(why) => panic!("couldn't create {}: {}", display, why),
                Ok(file) => file,
            };

            for query in &queries {
                file.write(query.as_bytes()).unwrap();
                file.write("\n".as_bytes()).unwrap();
            }

            println!("");
            write_message(
                String::from(format!(
                    "Successfully saved {} queries to {}",
                    queries.len(),
                    output
                ))
                .green(),
            );

            if skipped_files.len() > 0 {
                println!();
                write_message(
                    String::from(format!("Skipped {} files:", skipped_files.len())).yellow(),
                );

                for file in skipped_files {
                    let file_path = file.path.replace(&path, "");

                    write_message(
                        String::from(format!(
                            "{}: {}",
                            file_path,
                            format!("{}", file.reason).bold()
                        ))
                        .white(),
                    );
                }
            }
        }
        Command::SchemaStats {
            documents,
            schema,
            include_fragments,
            json,
        } => {
            let queries = read_to_string(documents).expect("Unable to read schema");

            let schema = generate_schema_stats(schema, queries, include_fragments);

            if json {
                let output =
                    serde_json::to_string_pretty(&schema).expect("Unable to convert stats to json");

                println!("{}", output);
            } else {
                for (name, object_type) in schema {
                    println!("");
                    write_message(
                        format!("{} {}", "type".white().italic(), name)
                            .magenta()
                            .bold(),
                    );

                    for (field_name, stats) in object_type.fields {
                        let name = if stats.count == 0 {
                            field_name.red()
                        } else {
                            field_name.green()
                        };

                        write_message(format!("  {} x {}", name, stats.count).normal());
                    }
                }
            }
        }
    }
}
