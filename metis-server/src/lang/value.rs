use metis_common::job_outputs::JobOutputPayload;
use metis_common::jobs::CreateJobRequestContext;

#[derive(Debug, Clone)]
pub enum Value {
    Int(i64),
    Float(f64),
    Bool(bool),
    Str(String),
    CodexOutput(JobOutputPayload),
    CodexContext(CreateJobRequestContext),
    // …
    // Function(Function),
    // NativeFunc(NativeFunc),
    // Object(Rc<RefCell<Object>>),
    Nil,
}

#[derive(Debug, Clone)]
pub enum RuntimeError {
    TypeMismatch {
        expected: &'static str,
        found: &'static str,
    },
    ArityMismatch {
        expected: usize,
        found: usize,
    },
    JobEngineError {
        reason: String,
    }, // …
}

pub trait IntoValue {
    fn into_value(self) -> Value;
}

pub trait FromValue: Sized {
    fn from_value(v: Value) -> Result<Self, RuntimeError>;
}

pub trait FromValueRef<'a>: Sized {
    fn from_value_ref(v: &'a Value) -> Result<Self, RuntimeError>;
}

macro_rules! impl_value_conversion {
    ($variant:ident, $ty:ty, $expected_name:expr) => {
        impl IntoValue for $ty {
            fn into_value(self) -> Value {
                Value::$variant(self)
            }
        }

        impl FromValue for $ty {
            fn from_value(v: Value) -> Result<Self, RuntimeError> {
                match v {
                    Value::$variant(inner) => Ok(inner),
                    other => Err(RuntimeError::TypeMismatch {
                        expected: $expected_name,
                        found: other.type_name(),
                    }),
                }
            }
        }

        impl<'a> FromValueRef<'a> for &'a $ty {
            fn from_value_ref(v: &'a Value) -> Result<Self, RuntimeError> {
                match v {
                    Value::$variant(inner) => Ok(inner),
                    other => Err(RuntimeError::TypeMismatch {
                        expected: $expected_name,
                        found: other.type_name(),
                    }),
                }
            }
        }
    };
}

// helper to pretty-print type names for errors
impl Value {
    pub fn type_name(&self) -> &'static str {
        match self {
            Value::Int(_) => "Int",
            Value::Float(_) => "Float",
            Value::Bool(_) => "Bool",
            Value::Str(_) => "Str",
            Value::CodexOutput(_) => "CodexOutput",
            Value::CodexContext(_) => "CodexContext",
            // …
            Value::Nil => "Nil",
        }
    }
}

impl_value_conversion!(Int, i64, "Int");
impl_value_conversion!(Float, f64, "Float");
impl_value_conversion!(Bool, bool, "Bool");
impl_value_conversion!(Str, String, "Str");
