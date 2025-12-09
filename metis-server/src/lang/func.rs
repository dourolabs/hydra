use crate::job_engine::{JobEngine, MetisId};
use crate::lang::value::{FromValueRef, RuntimeError, Value};

pub struct Args<'a> {
    pub vals: &'a [Value],
}

impl<'a> Args<'a> {
    pub fn len(&self) -> usize {
        self.vals.len()
    }

    pub fn get<T>(&self, idx: usize) -> Result<T, RuntimeError>
    where
        T: FromValueRef<'a>,
    {
        self.vals
            .get(idx)
            .ok_or(RuntimeError::ArityMismatch {
                expected: idx + 1,
                found: self.len(),
            })
            .and_then(|v| T::from_value_ref(v))
    }
}

fn builtin_add(args: &Args) -> Result<Value, RuntimeError> {
    let a: &i64 = args.get(0)?;
    let b: &i64 = args.get(1)?;
    Ok(Value::Int(*a + *b))
}

fn builtin_eq(args: &Args) -> Result<Value, RuntimeError> {
    let a: &i64 = args.get(0)?;
    let b: &i64 = args.get(1)?;
    Ok(Value::Bool(*a == *b))
}

fn builtin_strlen(args: &Args) -> Result<Value, RuntimeError> {
    let s: &String = args.get(0)?;
    Ok(Value::Int(s.len() as i64))
}

pub trait NativeFunc: Send + Sync {
    fn call(&self, args: &[Value]) -> Option<Result<Value, RuntimeError>>;

    fn spawn(&self, args: &[Value], id: MetisId, engine: &dyn JobEngine) -> Result<(), RuntimeError>;

    fn finalize(&self, args: &[Value], id: MetisId, engine: &dyn JobEngine) -> Result<Value, RuntimeError>;
}

impl<F> NativeFunc for F
where
    F: Fn(&[Value]) -> Result<Value, RuntimeError> + Send + Sync,
{
    fn call(&self, args: &[Value]) -> Option<Result<Value, RuntimeError>> {
        Some((self)(args))
    }

    fn spawn(&self, _args: &[Value], _id: MetisId, _engine: &dyn JobEngine) -> Result<(), RuntimeError> {
        Ok(())
    }

    fn finalize(&self, _args: &[Value], _id: MetisId, _engine: &dyn JobEngine) -> Result<Value, RuntimeError> {
        Ok(Value::Nil)
    }
}

#[derive(Clone)]
pub struct Builtin {
    pub name: &'static str,
    pub func: std::sync::Arc<dyn NativeFunc>,
}

impl std::fmt::Debug for Builtin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Builtin")
            .field("name", &self.name)
            .finish()
    }
}

impl PartialEq for Builtin {
    fn eq(&self, other: &Self) -> bool {
        self.name == other.name
    }
}

impl Eq for Builtin {}

impl Builtin {
    pub fn new(name: &'static str, func: impl NativeFunc + 'static) -> Self {
        Self {
            name,
            func: std::sync::Arc::new(func),
        }
    }
}

pub fn call_builtin(b: &dyn NativeFunc, args: &[Value]) -> Option<Result<Value, RuntimeError>> {
    let args = Args { vals: args };
    b.call(args.vals)
}

pub struct Codex {}

impl NativeFunc for Codex {
    fn call(&self, _args: &[Value]) -> Option<Result<Value, RuntimeError>> {
        None
    }

    fn spawn(&self, _args: &[Value], _id: MetisId, _engine: &dyn JobEngine) -> Result<(), RuntimeError> {
        // TODO
        Ok(())
    }

    fn finalize(&self, _args: &[Value], _id: MetisId, _engine: &dyn JobEngine) -> Result<Value, RuntimeError> {
        // TODO
        Ok(Value::Nil)
    }
}