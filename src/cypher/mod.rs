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
//! cypher.exec("create (n:CYPHER_QUERY {value: 1})").unwrap();
//!
//! let mut query = cypher.query();
//! query.add_statement("match (n:CYPHER_QUERY) return n.value as value");
//!
//! let result = query.send().unwrap();
//!
//! for row in result[0].rows() {
//!     let value: i32 = row.get("value").unwrap();
//!     assert_eq!(value, 1);
//! }
//! # cypher.exec("match (n:CYPHER_QUERY) delete n");
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
use std::error::Error;
use hyper::Url;
use hyper::header::Headers;
use hyper::client::{Client, Response};
use serde::Deserialize;
use serde_json::{self, Value};

use self::result::ResultTrait;
use ::error::{GraphError, Neo4jError};

fn send_query(client: &Client, endpoint: &str, headers: &Headers, statements: Vec<Statement>)
    -> Result<Response, GraphError> {

    let mut json = BTreeMap::new();
    json.insert("statements", statements);
    let json = try!(serde_json::to_string(&json));

    let req = client.post(endpoint)
        .headers(headers.clone())
        .body(&json);

    let res = try!(req.send());
    Ok(res)
}

fn parse_response<T: Deserialize + ResultTrait>(res: &mut Response) -> Result<T, GraphError> {
    let mut res = res;
    let value: Value = try!(serde_json::de::from_reader(&mut res));
    let result = try!(serde_json::value::from_value::<T>(value.clone()));

    if result.errors().len() > 0 {
        return Err(GraphError::new_neo4j_error(result.errors().clone()));
    }

    Ok(result)
}

#[derive(Debug, Deserialize)]
struct QueryResult {
    results: Vec<CypherResult>,
    errors: Vec<Neo4jError>,
}

impl ResultTrait for QueryResult {
    fn results(&self) -> &Vec<CypherResult> {
        &self.results
    }

    fn errors(&self) -> &Vec<Neo4jError> {
        &self.errors
    }
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

    /// Creates a new `CypherQuery`
    pub fn query(&self) -> CypherQuery {
        CypherQuery {
            statements: Vec::new(),
            cypher: &self,
        }
    }

    /// Executes a cypher query
    ///
    /// Parameter can be anything that implements `Into<Statement>`, `&str` or or `Statement` itself
    ///
    /// # Examples
    ///
    /// ```
    /// # use rusted_cypher::GraphClient;
    /// # let graph = GraphClient::connect("http://neo4j:neo4j@localhost:7474/db/data").unwrap();
    /// # let cypher = graph.cypher();
    /// let result = cypher.exec("match n return n");
    /// # let result = result.unwrap();
    /// # assert_eq!(result[0].columns.len(), 1);
    /// # assert_eq!(result[0].columns[0], "n");
    /// ```
    pub fn exec<S: Into<Statement>>(&self, statement: S) -> Result<Vec<CypherResult>, GraphError> {
        let mut query = self.query();
        query.add_statement(statement);

        query.send()
    }

    pub fn begin_transaction(&self, statements: Vec<Statement>) -> Result<(Transaction, Vec<CypherResult>), GraphError> {
        Transaction::begin(&format!("{}", &self.endpoint), &self.headers, statements)
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
    /// Adds a statement to the query
    ///
    /// The statement can be anything that implements Into<Statement>.
    pub fn add_statement<T: Into<Statement>>(&mut self, statement: T) {
        self.statements.push(statement.into());
    }

    pub fn set_statements(&mut self, statements: Vec<Statement>) {
        self.statements = statements;
    }

    /// Sends the query to the server
    ///
    /// The statements contained in the query are sent to the server and the results are parsed
    /// into a `Vec<CypherResult>` in order to match the response of the neo4j api. If there is an
    /// error, a `GraphError` is returned.
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

        // let result: Value = try!(serde_json::de::from_reader(&mut res));
        // match serde_json::value::from_value::<QueryResult>(result) {
        //     Ok(result) => {
        //         if result.errors.len() > 0 {
        //             return Err(GraphError::new_neo4j_error(result.errors))
        //         }
        //
        //         return Ok(result.results);
        //     }
        //     Err(e) => return Err(GraphError::new_error(Box::new(e)))
        // }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn query() {
        let cypher = get_cypher();
        let mut query = cypher.query();

        query.add_statement("match n return n");

        let result = query.send().unwrap();

        assert_eq!(result[0].columns.len(), 1);
        assert_eq!(result[0].columns[0], "n");
    }

    #[test]
    fn transaction() {
        let cypher = get_cypher();

        let stmt = Statement::new("create (n:CYPHER_TRANSACTION) return n");
        let (transaction, results) = cypher.begin_transaction(vec![stmt]).unwrap();

        assert_eq!(results[0].data.len(), 1);

        transaction.commit().unwrap();

        let stmt = Statement::new("match (n:CYPHER_TRANSACTION) delete n");
        let (transaction, _) = cypher.begin_transaction(vec![stmt]).unwrap();
        transaction.commit().unwrap();
    }
}