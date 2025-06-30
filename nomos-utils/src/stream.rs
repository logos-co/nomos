use async_trait::async_trait;
use futures::{Stream, StreamExt as _};

#[async_trait]
pub trait StreamExt<WrappedType>: Stream + Sized + Unpin
where
    WrappedType: InstantiateInnerType<Self, WrappedTypeInit: Send + 'static>,
{
    async fn into_wrapped_type(
        mut self,
        wrapped_type_init: WrappedType::WrappedTypeInit,
    ) -> WrappedType {
        let next_stream_item = async {
            loop {
                if let Some(next_item) = self.next().await {
                    break next_item;
                }
            }
        }
        .await;

        WrappedType::new(wrapped_type_init, next_stream_item, self)
    }
}

#[async_trait]
impl<S, WrappedType> StreamExt<WrappedType> for S
where
    S: Stream + Sized + Unpin,
    WrappedType: InstantiateInnerType<S, WrappedTypeInit: Send + 'static>,
{
}

pub trait InstantiateInnerType<InputStream>
where
    InputStream: Stream,
{
    type WrappedTypeInit;
    fn new(
        inner_type_init: Self::WrappedTypeInit,
        input_stream_item: InputStream::Item,
        input_stream: InputStream,
    ) -> Self;
}

#[cfg(test)]
mod tests {
    use futures::Stream;
    use tokio_stream::iter;

    use crate::stream::{InstantiateInnerType, StreamExt as _};

    struct TestStruct(bool);

    impl<InputStream> InstantiateInnerType<InputStream> for TestStruct
    where
        InputStream: Stream<Item = bool>,
    {
        type WrappedTypeInit = ();

        fn new(
            _inner_type_init: Self::WrappedTypeInit,
            input_stream_item: <InputStream as Stream>::Item,
            _input_stream: InputStream,
        ) -> Self {
            Self(input_stream_item)
        }
    }

    #[tokio::test]
    async fn initialize_true() {
        let test_struct: TestStruct = iter([true]).into_wrapped_type(()).await;
        assert!(test_struct.0);
    }

    #[tokio::test]
    async fn initialize_false() {
        let test_struct: TestStruct = iter([false]).into_wrapped_type(()).await;
        assert!(!test_struct.0);
    }
}
