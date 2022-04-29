use crate::cmd::*;
use crate::tr;
use log::error;

pub fn run_facet(args: &FacetArgs, app: Application) {
    let fts = app.open_book_reader().unwrap_or_else(|e| {
        error!("{}", e);
        std::process::exit(4);
    });
    match fts.get_facet(&args.path, None, false, Some(args.hits)) {
        Ok(res) => {
            println!("{}", serde_json::to_string(&res).unwrap());
        }
        Err(e) => {
            error!("{}: {}", tr!["Query error", "Ошибка запроса"], e);
            std::process::exit(2);
        }
    }
}
