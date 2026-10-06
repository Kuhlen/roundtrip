//! GraphQL request as edited. On disk it is a JSON body.

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct GraphqlBody {
    pub query: String,
    /// JSON text as typed; blank = none
    pub variables: String,
    pub operation_name: String,
}
