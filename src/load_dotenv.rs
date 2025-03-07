use crate::common::*;

pub(crate) fn load_dotenv() -> RunResult<'static, BTreeMap<String, String>> {
  match dotenv::dotenv_iter() {
    Ok(iter) => {
      let result: dotenv::Result<BTreeMap<String, String>> = iter.collect();
      result.map_err(|dotenv_error| RuntimeError::Dotenv { dotenv_error })
    }
    Err(dotenv_error) => {
      if dotenv_error.not_found() {
        Ok(BTreeMap::new())
      } else {
        Err(RuntimeError::Dotenv { dotenv_error })
      }
    }
  }
}
