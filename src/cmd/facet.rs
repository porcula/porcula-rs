use crate::cmd::*;

pub fn run_facet(args: &FacetArgs, app: Application) -> ProcessResult {
    let fts = match app.open_book_reader() {
        Ok(x) => x,
        Err(e) => return ProcessResult::IndexError(e),
    };
    match fts.get_facet(&args.path, None, false, false, Some(args.hits)) {
        Ok(res) => {
            println!("{}", serde_json::to_string(&res).unwrap());
            ProcessResult::Ok
        }
        Err(e) => ProcessResult::QueryError(e.to_string()),
    }
}
