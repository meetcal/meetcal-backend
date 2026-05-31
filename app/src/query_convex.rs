use std::collections::BTreeMap;

use anyhow::{Context, Error, Result, bail};
use convex::{ConvexClient, FunctionResult, Value};
use serde::de::DeserializeOwned;

pub async fn get_convex_response<T: DeserializeOwned>(
    client: &ConvexClient,
    convex_func_name: &str,
    query_args: BTreeMap<String, Value>,
) -> Result<T, Error> {
    let mut client = client.clone();

    let convex_result = client.query(convex_func_name, query_args).await?;

    let parsed_result: T = match convex_result {
        // convex returns value not string so use serde to parse
        FunctionResult::Value(val) => {
            let json_value = val.export();
            serde_json::from_value(json_value).context("Failed to parse from convex response")?
        }
        // bail returns error we can handle vs panic would crash and quit
        FunctionResult::ErrorMessage(err) => bail!(err),
        FunctionResult::ConvexError(err) => bail!("ConvexError: {err:?}"),
    };

    Ok(parsed_result)
}
