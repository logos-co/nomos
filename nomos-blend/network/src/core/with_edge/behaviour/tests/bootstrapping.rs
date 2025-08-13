use test_log::test;

#[test(tokio::test)]
async fn edge_peer_not_supporting_blend() {}

#[test(tokio::test)]
async fn peer_not_an_edge_node() {}

#[test(tokio::test)]
async fn incoming_connection_with_maximum_peering_degree() {}

#[test(tokio::test)]
async fn concurrent_incoming_connection_and_maximum_peering_degree_reached() {}

#[test(tokio::test)]
async fn outgoing_connection_to_edge_peer() {}
