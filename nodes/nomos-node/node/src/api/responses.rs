pub mod ndjson {
    use axum::{
        body::Body,
        response::{IntoResponse as _, Response},
    };
    use futures::{Stream, StreamExt as _};
    use http::StatusCode;
    use nomos_api::http::DynError;
    use serde::Serialize;

    pub fn from_stream_result<T>(
        stream: Result<impl Stream<Item = Result<T, DynError>> + Send + 'static, DynError>,
    ) -> Response
    where
        T: Serialize,
    {
        match stream {
            Ok(stream) => from_stream(stream),
            Err(error) => (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()).into_response(),
        }
    }

    pub fn from_stream<T>(
        stream: impl Stream<Item = Result<T, DynError>> + Send + 'static,
    ) -> Response
    where
        T: Serialize,
    {
        let stream = stream.map(|item| {
            let data = item?;
            let mut bytes =
                serde_json::to_vec(&data).map_err(|error| Box::new(error) as DynError)?;
            bytes.push(b'\n');
            Ok::<_, DynError>(bytes)
        });

        let stream_body = Body::from_stream(stream);

        Response::builder()
            .status(StatusCode::OK)
            .header("content-type", "application/x-ndjson")
            .body(stream_body)
            .unwrap()
            .into_response()
    }
}

#[cfg(test)]
mod tests {
    use std::pin::Pin;

    use axum::body;
    use futures::{Stream, stream};
    use http::StatusCode;
    use nomos_api::http::DynError;
    use serde::Serialize;

    use super::*;

    #[derive(Serialize)]
    struct TestData {
        value: i32,
    }

    #[tokio::test]
    async fn test_from_stream_ok() {
        let test_data = vec![TestData { value: 1 }, TestData { value: 2 }];
        let stream = stream::iter(test_data.into_iter().map(Ok::<_, DynError>));
        let response = ndjson::from_stream_result(Ok(stream));

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.headers().get("content-type").unwrap(),
            "application/x-ndjson"
        );

        let body_bytes = body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        assert_eq!(
            body_bytes.iter().as_slice(),
            b"{\"value\":1}\n{\"value\":2}\n"
        );
    }

    #[tokio::test]
    async fn test_error_stream() {
        let error = "Test error";
        let response = ndjson::from_stream_result::<TestData>(Err::<
            Pin<Box<dyn Stream<Item = Result<TestData, DynError>> + Send>>,
            DynError,
        >(Box::new(
            std::io::Error::other(error),
        ) as DynError));

        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        let body_bytes = body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        assert_eq!(body_bytes.iter().as_slice(), b"Test error");
    }
}
