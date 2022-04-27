use crate::cmd::*;
use crate::tr;

pub fn run_facet(args: &FacetArgs, app: Application) {
    let fts = app.open_book_reader().unwrap_or_else(|e| {
        eprintln!("{}", e);
        std::process::exit(4);
    });
    match fts.get_facet(&args.path, None, false, Some(args.hits), app.debug) {
        Ok(res) => {
            println!("{}", serde_json::to_string(&res).unwrap());
        }
        Err(e) => {
            eprintln!("{}: {}", tr!["Query error", "Ошибка запроса"], e);
            std::process::exit(2);
        }
    }
}
