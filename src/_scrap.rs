fn get_query_from_log(line: String) -> Option<stats::Query> {
    let v: Vec<&str> = line.split("query:").collect();

    if v.len() < 2 {
        return None;
    }

    lazy_static! {
        static ref QUERY_HASH_RE: Regex = Regex::new(r"query_hash=(?P<hash>[a-z0-9]+)").unwrap();
    }

    let metadata = String::from(v[0]);
    let query = String::from(v[1].trim());

    match QUERY_HASH_RE.captures(&metadata).unwrap().name("hash") {
        Some(hash) => {
            return Some(stats::Query {
                query: query,
                hash: hash.as_str().to_string(),
                count: 0,
            });
        }
        None => return None,
    }
}
