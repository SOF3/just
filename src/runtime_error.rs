use crate::common::*;

#[derive(Debug)]
pub(crate) enum RuntimeError<'a> {
  ArgumentCountMismatch {
    recipe: &'a str,
    parameters: Vec<&'a Parameter<'a>>,
    found: usize,
    min: usize,
    max: usize,
  },
  Backtick {
    token: Token<'a>,
    output_error: OutputError,
  },
  Code {
    recipe: &'a str,
    line_number: Option<usize>,
    code: i32,
  },
  Cygpath {
    recipe: &'a str,
    output_error: OutputError,
  },
  Dotenv {
    dotenv_error: dotenv::Error,
  },
  FunctionCall {
    token: Token<'a>,
    message: String,
  },
  Internal {
    message: String,
  },
  IoError {
    recipe: &'a str,
    io_error: io::Error,
  },
  Shebang {
    recipe: &'a str,
    command: String,
    argument: Option<String>,
    io_error: io::Error,
  },
  Signal {
    recipe: &'a str,
    line_number: Option<usize>,
    signal: i32,
  },
  TmpdirIoError {
    recipe: &'a str,
    io_error: io::Error,
  },
  UnknownOverrides {
    overrides: Vec<&'a str>,
  },
  UnknownRecipes {
    recipes: Vec<&'a str>,
    suggestion: Option<&'a str>,
  },
  Unknown {
    recipe: &'a str,
    line_number: Option<usize>,
  },
}

impl<'a> RuntimeError<'a> {
  pub(crate) fn code(&self) -> Option<i32> {
    use RuntimeError::*;
    match *self {
      Code { code, .. }
      | Backtick {
        output_error: OutputError::Code(code),
        ..
      } => Some(code),
      _ => None,
    }
  }
}

impl<'a> Display for RuntimeError<'a> {
  fn fmt(&self, f: &mut Formatter) -> Result<(), fmt::Error> {
    use RuntimeError::*;

    let color = if f.alternate() {
      Color::always()
    } else {
      Color::never()
    };
    let error = color.error();
    let message = color.message();
    write!(f, "{} {}", error.paint("error:"), message.prefix())?;

    let mut error_token = None;

    match *self {
      UnknownRecipes {
        ref recipes,
        ref suggestion,
      } => {
        write!(
          f,
          "Justfile does not contain {} {}.",
          Count("recipe", recipes.len()),
          List::or_ticked(recipes),
        )?;
        if let Some(suggestion) = *suggestion {
          write!(f, "\nDid you mean `{}`?", suggestion)?;
        }
      }
      UnknownOverrides { ref overrides } => {
        write!(
          f,
          "{} {} overridden on the command line but not present in justfile",
          Count("Variable", overrides.len()),
          List::and_ticked(overrides),
        )?;
      }
      ArgumentCountMismatch {
        recipe,
        ref parameters,
        found,
        min,
        max,
      } => {
        if min == max {
          let expected = min;
          write!(
            f,
            "Recipe `{}` got {} {} but {}takes {}",
            recipe,
            found,
            Count("argument", found),
            if expected < found { "only " } else { "" },
            expected
          )?;
        } else if found < min {
          write!(
            f,
            "Recipe `{}` got {} {} but takes at least {}",
            recipe,
            found,
            Count("argument", found),
            min
          )?;
        } else if found > max {
          write!(
            f,
            "Recipe `{}` got {} {} but takes at most {}",
            recipe,
            found,
            Count("argument", found),
            max
          )?;
        }
        write!(f, "\nusage:\n    just {}", recipe)?;
        for param in parameters {
          if color.stderr().active() {
            write!(f, " {:#}", param)?;
          } else {
            write!(f, " {}", param)?;
          }
        }
      }
      Code {
        recipe,
        line_number,
        code,
      } => {
        if let Some(n) = line_number {
          write!(
            f,
            "Recipe `{}` failed on line {} with exit code {}",
            recipe, n, code
          )?;
        } else {
          write!(f, "Recipe `{}` failed with exit code {}", recipe, code)?;
        }
      }
      Cygpath {
        recipe,
        ref output_error,
      } => match *output_error {
        OutputError::Code(code) => {
          write!(
            f,
            "Cygpath failed with exit code {} while translating recipe `{}` \
             shebang interpreter path",
            code, recipe
          )?;
        }
        OutputError::Signal(signal) => {
          write!(
            f,
            "Cygpath terminated by signal {} while translating recipe `{}` \
             shebang interpreter path",
            signal, recipe
          )?;
        }
        OutputError::Unknown => {
          write!(
            f,
            "Cygpath experienced an unknown failure while translating recipe `{}` \
             shebang interpreter path",
            recipe
          )?;
        }
        OutputError::Io(ref io_error) => {
          match io_error.kind() {
            io::ErrorKind::NotFound => write!(
              f,
              "Could not find `cygpath` executable to translate recipe `{}` \
               shebang interpreter path:\n{}",
              recipe, io_error
            ),
            io::ErrorKind::PermissionDenied => write!(
              f,
              "Could not run `cygpath` executable to translate recipe `{}` \
               shebang interpreter path:\n{}",
              recipe, io_error
            ),
            _ => write!(f, "Could not run `cygpath` executable:\n{}", io_error),
          }?;
        }
        OutputError::Utf8(ref utf8_error) => {
          write!(
            f,
            "Cygpath successfully translated recipe `{}` shebang interpreter path, \
             but output was not utf8: {}",
            recipe, utf8_error
          )?;
        }
      },
      Dotenv { ref dotenv_error } => {
        writeln!(f, "Failed to load .env: {}", dotenv_error)?;
      }
      FunctionCall {
        ref token,
        ref message,
      } => {
        writeln!(
          f,
          "Call to function `{}` failed: {}",
          token.lexeme(),
          message
        )?;
        error_token = Some(token);
      }
      Shebang {
        recipe,
        ref command,
        ref argument,
        ref io_error,
      } => {
        if let Some(ref argument) = *argument {
          write!(
            f,
            "Recipe `{}` with shebang `#!{} {}` execution error: {}",
            recipe, command, argument, io_error
          )?;
        } else {
          write!(
            f,
            "Recipe `{}` with shebang `#!{}` execution error: {}",
            recipe, command, io_error
          )?;
        }
      }
      Signal {
        recipe,
        line_number,
        signal,
      } => {
        if let Some(n) = line_number {
          write!(
            f,
            "Recipe `{}` was terminated on line {} by signal {}",
            recipe, n, signal
          )?;
        } else {
          write!(f, "Recipe `{}` was terminated by signal {}", recipe, signal)?;
        }
      }
      Unknown {
        recipe,
        line_number,
      } => {
        if let Some(n) = line_number {
          write!(
            f,
            "Recipe `{}` failed on line {} for an unknown reason",
            recipe, n
          )?;
        } else {
        }
      }
      IoError {
        recipe,
        ref io_error,
      } => {
        match io_error.kind() {
          io::ErrorKind::NotFound => writeln!(
            f,
            "Recipe `{}` could not be run because just could not find `sh`:{}",
            recipe, io_error
          ),
          io::ErrorKind::PermissionDenied => writeln!(
            f,
            "Recipe `{}` could not be run because just could not run `sh`:{}",
            recipe, io_error
          ),
          _ => writeln!(
            f,
            "Recipe `{}` could not be run because of an IO error while \
             launching `sh`:{}",
            recipe, io_error
          ),
        }?;
      }
      TmpdirIoError {
        recipe,
        ref io_error,
      } => writeln!(
        f,
        "Recipe `{}` could not be run because of an IO error while trying \
         to create a temporary directory or write a file to that directory`:{}",
        recipe, io_error
      )?,
      Backtick {
        ref token,
        ref output_error,
      } => match *output_error {
        OutputError::Code(code) => {
          writeln!(f, "Backtick failed with exit code {}", code)?;
          error_token = Some(token);
        }
        OutputError::Signal(signal) => {
          writeln!(f, "Backtick was terminated by signal {}", signal)?;
          error_token = Some(token);
        }
        OutputError::Unknown => {
          writeln!(f, "Backtick failed for an unknown reason")?;
          error_token = Some(token);
        }
        OutputError::Io(ref io_error) => {
          match io_error.kind() {
            io::ErrorKind::NotFound => write!(
              f,
              "Backtick could not be run because just could not find `sh`:\n{}",
              io_error
            ),
            io::ErrorKind::PermissionDenied => write!(
              f,
              "Backtick could not be run because just could not run `sh`:\n{}",
              io_error
            ),
            _ => write!(
              f,
              "Backtick could not be run because of an IO \
               error while launching `sh`:\n{}",
              io_error
            ),
          }?;
          error_token = Some(token);
        }
        OutputError::Utf8(ref utf8_error) => {
          writeln!(
            f,
            "Backtick succeeded but stdout was not utf8: {}",
            utf8_error
          )?;
          error_token = Some(token);
        }
      },
      Internal { ref message } => {
        write!(
          f,
          "Internal runtime error, this may indicate a bug in just: {} \
           consider filing an issue: https://github.com/casey/just/issues/new",
          message
        )?;
      }
    }

    write!(f, "{}", message.suffix())?;

    if let Some(token) = error_token {
      write_message_context(
        f,
        Color::fmt(f).error(),
        token.text,
        token.offset,
        token.line,
        token.column,
        token.lexeme().len(),
      )?;
    }

    Ok(())
  }
}
