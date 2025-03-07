use crate::common::*;

pub(crate) enum SearchError {
  MultipleCandidates {
    candidates: Vec<PathBuf>,
  },
  Io {
    directory: PathBuf,
    io_error: io::Error,
  },
  NotFound,
}

impl fmt::Display for SearchError {
  fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
    match self {
      SearchError::Io {
        directory,
        io_error,
      } => write!(
        f,
        "I/O error reading directory `{}`: {}",
        directory.display(),
        io_error
      ),
      SearchError::MultipleCandidates { candidates } => write!(
        f,
        "Multiple candidate justfiles found in `{}`: {}",
        candidates[0].parent().unwrap().display(),
        List::and_ticked(
          candidates
            .iter()
            .map(|candidate| candidate.file_name().unwrap().to_string_lossy())
        ),
      ),
      SearchError::NotFound => write!(f, "No justfile found"),
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn multiple_candidates_formatting() {
    let error = SearchError::MultipleCandidates {
      candidates: vec![
        PathBuf::from("/foo/justfile"),
        PathBuf::from("/foo/JUSTFILE"),
      ],
    };

    assert_eq!(
      error.to_string(),
      "Multiple candidate justfiles found in `/foo`: `justfile` and `JUSTFILE`"
    )
  }
}
