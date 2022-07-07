use std::{borrow::Cow, convert::Infallible, error::Error as StdError, fmt, str::FromStr};

#[derive(Debug, Clone)]
pub(crate) struct RoutingError {
    reason: Cow<'static, str>,
}

impl RoutingError {
    pub fn new<S>(reason: S) -> Self
    where
        S: Into<Cow<'static, str>>,
    {
        Self {
            reason: reason.into(),
        }
    }
}

impl FromStr for RoutingError {
    type Err = Infallible;

    fn from_str(reason: &str) -> Result<Self, Self::Err> {
        Ok(Self {
            reason: reason.to_owned().into(),
        })
    }
}

impl fmt::Display for RoutingError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.reason)
    }
}

impl StdError for RoutingError {}

macro_rules! bail {
    ($err:expr $(,)?) => {
        return Err(RoutingError::new(err));
    };
    ($fmt:expr, $($arg:tt)*) => {
        return Err(RoutingError::new(format!(fmt, $(arg)*)))
    };
}

macro_rules! format_err {
    ($err:expr $(,)?) => {
        RoutingError::new(err)
    };
    ($fmt:expr, $($arg:tt)*) => {
        RoutingError::new(format!(fmt, $(arg)*))
    };
}
