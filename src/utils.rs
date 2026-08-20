use std::pin::Pin;

#[cfg(not(target_arch = "wasm32"))]
pub trait ConditionalSend: Send {}

#[cfg(not(target_arch = "wasm32"))]
impl<T: Send> ConditionalSend for T {}

#[cfg(target_arch = "wasm32")]
pub trait ConditionalSend {}

#[cfg(target_arch = "wasm32")]
impl<T> ConditionalSend for T {}

pub trait ConditionalSendFuture: Future + ConditionalSend {}

impl<T: Future + ConditionalSend> ConditionalSendFuture for T {}

#[cfg(not(target_arch = "wasm32"))]
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

#[cfg(target_arch = "wasm32")]
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + 'a>>;
