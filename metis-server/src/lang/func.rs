use crate::job_engine::{JobEngine, MetisId};
use crate::lang::value::{FromValueRef, RuntimeError, Value};
use async_trait::async_trait;

pub struct Args {
    pub vals: Vec<Value>,
}

impl Args {
    pub fn len(&self) -> usize {
        self.vals.len()
    }

    pub fn get<'a, T>(&'a self, idx: usize) -> Result<T, RuntimeError>
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

    pub fn from_slice(vals: &[Value]) -> Self {
        Self {
            vals: vals.to_vec(),
        }
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

#[async_trait]
pub trait NativeFunc: Send + Sync {
    fn call(&self, args: &Args) -> Option<Result<Value, RuntimeError>>;

    async fn spawn(
        &self,
        args: &Args,
        id: MetisId,
        engine: &dyn JobEngine,
    ) -> Result<(), RuntimeError>;

    async fn finalize(
        &self,
        args: &Args,
        id: MetisId,
        engine: &dyn JobEngine,
    ) -> Result<Value, RuntimeError>;
}

#[async_trait]
impl<F> NativeFunc for F
where
    F: Fn(&Args) -> Result<Value, RuntimeError> + Send + Sync,
{
    fn call(&self, args: &Args) -> Option<Result<Value, RuntimeError>> {
        Some((self)(args))
    }

    async fn spawn(
        &self,
        _args: &Args,
        _id: MetisId,
        _engine: &dyn JobEngine,
    ) -> Result<(), RuntimeError> {
        Ok(())
    }

    async fn finalize(
        &self,
        _args: &Args,
        _id: MetisId,
        _engine: &dyn JobEngine,
    ) -> Result<Value, RuntimeError> {
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
        f.debug_struct("Builtin").field("name", &self.name).finish()
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
    let args = Args::from_slice(args);
    b.call(&args)
}

pub struct Codex {}

#[async_trait]
impl NativeFunc for Codex {
    fn call(&self, _args: &Args) -> Option<Result<Value, RuntimeError>> {
        None
    }

    async fn spawn(
        &self,
        args: &Args,
        id: MetisId,
        engine: &dyn JobEngine,
    ) -> Result<(), RuntimeError> {
        let prompt: &String = args.get(0)?;

        match engine.create_job(&id, prompt).await {
            Ok(()) => Ok(()),
            Err(err) => Err(RuntimeError::JobEngineError {
                reason: format!("Failed to create Kubernetes job: {err}"),
            }),
        }
    }

    async fn finalize(
        &self,
        _args: &Args,
        _id: MetisId,
        _engine: &dyn JobEngine,
    ) -> Result<Value, RuntimeError> {
        // TODO
        Ok(Value::Nil)
    }
}
