use crate::cmd::*;

pub fn run_query(args: &QueryArgs, app: Application) -> ProcessResult {
    let fts = match app.open_book_reader() {
        Ok(x) => x,
        Err(e) => return ProcessResult::IndexError(e),
    };
    match fts.search_as_json(
        &args.query,
        args.stem,
        args.disjunction,
        crate::fts::OrderBy::Default,
        args.hits,
        0,
    ) {
        Ok(res) => {
            println!("{res}");
            ProcessResult::Ok
        }
        Err(e) => ProcessResult::QueryError(e.to_string()),
    }
}
