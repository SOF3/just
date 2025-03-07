use crate::common::*;

pub(crate) struct Variables<'a> {
  stack: Vec<&'a Expression<'a>>,
}

impl<'a> Variables<'a> {
  pub(crate) fn new(root: &'a Expression<'a>) -> Variables<'a> {
    Variables { stack: vec![root] }
  }
}

impl<'a> Iterator for Variables<'a> {
  type Item = &'a Token<'a>;

  fn next(&mut self) -> Option<&'a Token<'a>> {
    match self.stack.pop() {
      None
      | Some(Expression::String { .. })
      | Some(Expression::Backtick { .. })
      | Some(Expression::Call { .. }) => None,
      Some(Expression::Variable { token, .. }) => Some(token),
      Some(Expression::Concatination { lhs, rhs }) => {
        self.stack.push(lhs);
        self.stack.push(rhs);
        self.next()
      }
      Some(Expression::Group { expression }) => {
        self.stack.push(expression);
        self.next()
      }
    }
  }
}
