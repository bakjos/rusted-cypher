//! Provides structs used to interact with the cypher transaction endpoint
//!
//! The types declared in this module, save for `Statement`, don't need to be instantiated
//! directly, since they can be obtained from the `GraphClient`
//!
//! # Examples
//!
//! ```
//! # extern crate hyper;
//! # extern crate rusted_cypher;
//! # use std::collections::BTreeMap;
//! # use hyper::Url;
//! # use hyper::header::{Authorization, Basic, ContentType, Headers};
//! # use rusted_cypher::cypher::Cypher;
//! # fn main() {
//! # let url = Url::parse("http://localhost:7474/db/data/transaction").unwrap();
//! #
//! # let mut headers = Headers::new();
//! # headers.set(Authorization(
//! #     Basic {
//! #         username: "neo4j".to_owned(),
//! #         password: Some("neo4j".to_owned()),
//! #     }
//! # ));
//! #
//! # headers.set(ContentType::json());
//!
//! let cypher = Cypher::new(url, headers);
//!
//! cypher.exec("CREATE (n:CYPHER_QUERY {value: 1})").unwrap();
//!
//! let mut query = cypher.query();
//! query.add_statement("MATCH (n:CYPHER_QUERY) RETURN n.value as value");
//!
//! let result = query.send().unwrap();
//!
//! for row in result[0].rows() {
//!     let value: i32 = row.get("value").unwrap();
//!     assert_eq!(value, 1);
//! }
//! # cypher.exec("MATCH (n:CYPHER_QUERY) delete n");
//! # }
//! ```

pub mod transaction;
pub mod statement;
pub mod result;

pub use self::statement::Statement;
pub use self::transaction::Transaction;
pub use self::result::CypherResult;

use std::convert::Into;
use std::collections::BTreeMap;
use hyper::client::{Client, Response};
use hyper::header::Headers;
use url::Url;
use serde::Deserialize;
use serde_json::{self, Value};
use serde_json::de as json_de;
use serde_json::value as json_value;

use self::result::{QueryResult, ResultTrait};
use ::error::GraphError;

fn send_query(client: &Client, endpoint: &str, headers: &Headers, statements: Vec<Statement>)
    -> Result<Response, GraphError> {

    let mut json = BTreeMap::new();
    json.insert("statements", statements);
    let json = match serde_json::to_string(&json) {
        Ok(json) => json,
        Err(e) => {
            error!("Unable to serialize request: {}", e);
            return Err(GraphError::new_error(Box::new(e)));
        }
    };

    let req = client.post(endpoint)
        .headers(headers.clone())
        .body(&json);

    debug!("Seding query:\n{}", ::serde_json::ser::to_string_pretty(&json).unwrap_or(String::new()));

    let res = try!(req.send());
    Ok(res)
}

fn parse_response<T: Deserialize + ResultTrait>(res: &mut Response) -> Result<T, GraphError> {
    let value = json_de::from_reader(res);
    let result = match value.and_then(|v: Value| json_value::from_value::<T>(v.clone())) {
        Ok(result) => result,
        Err(e) => {
            error!("Unable to parse response: {}", e);
            return Err(GraphError::new_error(Box::new(e)));
        }
    };

    if result.errors().len() > 0 {
        return Err(GraphError::new_neo4j_error(result.errors().clone()));
    }

    Ok(result)
}

/// Represents the cypher endpoint of a neo4j server
///
/// The `Cypher` struct holds information about the cypher enpoint. It is used to create the queries
/// that are sent to the server.
pub struct Cypher {
    endpoint: Url,
    client: Client,
    headers: Headers,
}

impl Cypher {
    /// Creates a new Cypher
    ///
    /// Its arguments are the cypher transaction endpoint and the HTTP headers containing HTTP
    /// Basic Authentication, if needed.
    pub fn new(endpoint: Url, headers: Headers) -> Self {
        Cypher {
            endpoint: endpoint,
            client: Client::new(),
            headers: headers,
        }
    }

    fn endpoint(&self) -> &Url {
        &self.endpoint
    }

    fn client(&self) -> &Client {
        &self.client
    }

    fn headers(&self) -> &Headers {
        &self.headers
    }

    pub fn query(&self) -> CypherQuery {
        CypherQuery {
            statements: Vec::new(),
            cypher: &self,
        }
    }

    /// Executes the given `Statement`
    ///
    /// Parameter can be anything that implements `Into<Statement>`, `&str` or `Statement` itself
    ///
    /// # Examples
    ///
    /// ```
    /// # use rusted_cypher::GraphClient;
    /// # use rusted_cypher::Statement;
    /// # let graph = GraphClient::connect("http://neo4j:neo4j@localhost:7474/db/data").unwrap();
    /// # let cypher = graph.cypher();
    /// // `&str` implements `Into<Statement>`
    /// let result = cypher.exec("MATCH n RETURN n");
    /// # let result = result.unwrap();
    /// # assert_eq!(result.columns.len(), 1);
    /// # assert_eq!(result.columns[0], "n");
    /// // Creating `Statement` by hand
    /// let statement = Statement::new("MATCH n RETURN n");
    /// let result = cypher.exec(statement);
    /// # let result = result.unwrap();
    /// # assert_eq!(result.columns.len(), 1);
    /// # assert_eq!(result.columns[0], "n");
    /// ```
    pub fn exec<S: Into<Statement>>(&self, statement: S) -> Result<CypherResult, GraphError> {
        let mut query = self.query();
        query.add_statement(statement);

        let mut results = try!(query.send());

        match results.pop() {
            Some(result) => Ok(result),
            None => Err(GraphError::new("No results returned from server")),
        }
    }

    pub fn transaction(&self) -> Transaction<self::transaction::Created> {
        Transaction::new(&self.endpoint.to_string(), &self.headers)
    }
}

/// Represents a cypher query
///
/// A cypher query is composed by statements, each one containing the query itself and its parameters.
///
/// The query parameters must implement `Serialize` so they can be serialized into JSON in order to
/// be sent to the server
pub struct CypherQuery<'a> {
    statements: Vec<Statement>,
    cypher: &'a Cypher,
}

impl<'a> CypherQuery<'a> {
    /// Adds statements in builder like style
    pub fn with_statement<T: Into<Statement>>(mut self, statement: T) -> Self {
        self.add_statement(statement);
        self
    }

    pub fn add_statement<T: Into<Statement>>(&mut self, statement: T) {
        self.statements.push(statement.into());
    }

    pub fn set_statements(&mut self, statements: Vec<Statement>) {
        self.statements = statements;
    }

    /// Sends the query to the server
    ///
    /// The statements contained in the query are sent to the server and the results are parsed
    /// into a `Vec<CypherResult>` in order to match the response of the neo4j api.
    pub fn send(self) -> Result<Vec<CypherResult>, GraphError> {
        let client = self.cypher.client();
        let endpoint = format!("{}/{}", self.cypher.endpoint(), "commit");
        let headers = self.cypher.headers();
        let mut res = try!(send_query(client, &endpoint, headers, self.statements));

        let result: QueryResult = try!(parse_response(&mut res));
        if result.errors().len() > 0 {
            return Err(GraphError::new_neo4j_error(result.errors().clone()))
        }

        Ok(result.results)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ::cypher::result::Row;

    #[derive(Serialize, Deserialize)]
    struct ComplexType {
        pub name: String,
        pub  value: i32,
    }

    fn get_cypher() -> Cypher {
        use hyper::Url;
        use hyper::header::{Authorization, Basic, ContentType, Headers};

        let cypher_endpoint = Url::parse("http://localhost:7474/db/data/transaction").unwrap();

        let mut headers = Headers::new();
        headers.set(Authorization(
            Basic {
                username: "neo4j".to_owned(),
                password: Some("neo4j".to_owned()),
            }
        ));
        headers.set(ContentType::json());

        Cypher::new(cypher_endpoint, headers)
    }

    #[test]
    fn query_without_params() {
        let result = get_cypher().exec("MATCH (n:TEST_CYPHER) RETURN n").unwrap();

        assert_eq!(result.columns.len(), 1);
        assert_eq!(result.columns[0], "n");
    }

    #[test]
    fn query_with_string_param() {
        let statement = Statement::new("MATCH (n:TEST_CYPHER {name: {name}}) RETURN n")
            .with_param("name", "Neo");

        let result = get_cypher().exec(statement).unwrap();

        assert_eq!(result.columns.len(), 1);
        assert_eq!(result.columns[0], "n");
    }

    #[test]
    fn query_with_int_param() {
        let statement = Statement::new("MATCH (n:TEST_CYPHER {value: {value}}) RETURN n")
            .with_param("value", 42);

        let result = get_cypher().exec(statement).unwrap();

        assert_eq!(result.columns.len(), 1);
        assert_eq!(result.columns[0], "n");
    }

    #[test]
    fn query_with_complex_param() {
        let cypher = get_cypher();

        let complex_param = ComplexType {
            name: "Complex".to_owned(),
            value: 42,
        };

        let statement = Statement::new("CREATE (n:TEST_CYPHER_COMPLEX_PARAM {p})")
            .with_param("p", &complex_param);

        let result = cypher.exec(statement);
        assert!(result.is_ok());

        let results = cypher.exec("MATCH (n:TEST_CYPHER_COMPLEX_PARAM) RETURN n").unwrap();
        let rows: Vec<Row> = results.rows().take(1).collect();
        let row = rows.first().unwrap();

        let complex_result: ComplexType = row.get("n").unwrap();
        assert_eq!(complex_result.name, "Complex");
        assert_eq!(complex_result.value, 42);

        cypher.exec("MATCH (n:TEST_CYPHER_COMPLEX_PARAM) DELETE n").unwrap();
    }

    #[test]
    fn query_with_multiple_params() {
        let statement = Statement::new("MATCH (n:TEST_CYPHER {name: {name}}) WHERE n.value = {value} RETURN n")
            .with_param("name", "Neo")
            .with_param("value", 42);

        let result = get_cypher().exec(statement).unwrap();
        assert_eq!(result.columns.len(), 1);
        assert_eq!(result.columns[0], "n");
    }

    #[test]
    fn multiple_queries() {
        let cypher = get_cypher();
        let statement1 = Statement::new("MATCH (n:TEST_CYPHER) RETURN n");
        let statement2 = Statement::new("MATCH (n:TEST_CYPHER) RETURN n");

        let query = cypher.query()
            .with_statement(statement1)
            .with_statement(statement2);

        let results = query.send().unwrap();
        assert_eq!(results.len(), 2);
    }
}
