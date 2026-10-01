use super::*;
use crate::protocol::{PROTOCOL_ARC_2809, RESULT_CODE_MORE_PAGES};

fn fixture() -> serde_json::Value {
    serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/fixtures/github_issue_reports.json"
    )))
    .unwrap()
}

fn payload(path: &[&str]) -> Vec<u8> {
    let document = fixture();
    let mut value = &document;
    for component in path {
        value = &value[*component];
    }
    decode_hexadecimal(value["payload"].as_str().unwrap())
}

#[test]
fn issue_52_tesira_partial_channel_pages_are_valid_and_explicitly_incomplete() {
    let transmitter_response = payload(&["issue_52", "tesira_transmitter_channel_partial_page"]);
    let transmitter_page =
        parse_modern_arc_transmitter_channel_status_page(&transmitter_response).unwrap();
    assert_eq!(transmitter_page.protocol_id, PROTOCOL_ARC_2809);
    assert_eq!(transmitter_page.result_code, RESULT_CODE_MORE_PAGES);
    assert_eq!(
        transmitter_page.page_disposition,
        ModernArcPageDisposition::MorePages
    );
    assert_eq!(transmitter_page.page_capacity, 32);
    assert_eq!(transmitter_page.reported_record_count, 28);
    assert_eq!(transmitter_page.records.first().unwrap().channel_number, 1);
    assert_eq!(transmitter_page.records.last().unwrap().channel_number, 28);

    let receiver_response = payload(&["issue_52", "tesira_receiver_channel_partial_page"]);
    let receiver_page = parse_modern_arc_receiver_channel_status_page(&receiver_response).unwrap();
    assert_eq!(receiver_page.protocol_id, PROTOCOL_ARC_2809);
    assert_eq!(receiver_page.result_code, RESULT_CODE_MORE_PAGES);
    assert_eq!(
        receiver_page.page_disposition,
        ModernArcPageDisposition::MorePages
    );
    assert_eq!(receiver_page.page_capacity, 16);
    assert_eq!(receiver_page.reported_record_count, 16);
    assert_eq!(receiver_page.records.first().unwrap().channel_number, 1);
    assert_eq!(receiver_page.records.last().unwrap().channel_number, 16);
}

#[test]
fn issue_52_unsupported_reply_remains_distinct_from_a_partial_page() {
    let response = payload(&["issue_52", "unsupported_mxwani4_receiver_channel_status"]);
    assert_eq!(parse_result_code(&response), Some(0x0030));
    assert_eq!(
        parse_modern_arc_receiver_channel_status_page(&response),
        None
    );
}

#[test]
fn issue_53_tesira_offset_8_is_not_used_as_the_audio_channel_count() {
    let response = payload(&["issue_53", "tesira_transmitter_flow_status"]);
    let page = parse_transmitter_flow_status_page(&response).unwrap();

    assert_eq!(page.reported_flow_count, 3);
    assert_eq!(
        page.flows
            .iter()
            .map(|flow| (
                flow.global_flow_id,
                flow.media_type_code,
                flow.media_local_flow_id
            ))
            .collect::<Vec<_>>(),
        [(1, 3, 1), (2, 3, 2), (3, 3, 3)]
    );
    assert_eq!(
        page.flows
            .iter()
            .map(|flow| flow.transmitter_channel_ids_by_slot.as_slice())
            .collect::<Vec<_>>(),
        [&[5, 6, 7, 8], &[1, 3, 2, 4], &[7, 8, 0, 0]]
    );
    assert_eq!(
        page.flows
            .iter()
            .map(|flow| flow.populated_slot_count)
            .collect::<Vec<_>>(),
        [4, 4, 2]
    );

    let offset_8_values = page
        .flows
        .iter()
        .map(|flow| read_u16(&response, usize::from(flow.record_pointer) + 8).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(offset_8_values, [1, 2, 3]);
    assert_ne!(
        offset_8_values,
        page.flows
            .iter()
            .map(|flow| flow.populated_slot_count)
            .collect::<Vec<_>>()
    );
}

fn receiver_flow_routing(page: &ReceiverFlowPage) -> Vec<Vec<Vec<u16>>> {
    page.flows
        .iter()
        .map(|flow| flow.receiver_channel_numbers_by_flow_channel.clone())
        .collect()
}

fn receiver_flow_endpoints(page: &ReceiverFlowPage) -> Vec<(u8, u16, Option<&str>)> {
    page.flows
        .iter()
        .flat_map(|flow| &flow.interface_endpoints)
        .map(|endpoint| {
            (
                endpoint.descriptor_length_bytes,
                endpoint.udp_port,
                endpoint.ipv4_address.as_deref(),
            )
        })
        .collect()
}

#[test]
fn issue_55_tesira_receiver_flows_use_four_word_bitmaps() {
    let page = parse_receiver_flow_page(&payload(&["issue_55", "tesira_receiver_flows"])).unwrap();

    assert_eq!(page.reported_flow_count, 3);
    assert!(page
        .flows
        .iter()
        .all(|flow| flow.receiver_bitmap_word_count == 4));
    assert_eq!(
        receiver_flow_routing(&page),
        [
            vec![vec![1], vec![2]],
            vec![vec![3], vec![4]],
            vec![vec![5], vec![6], vec![7], vec![8]],
        ]
    );
    assert_eq!(
        receiver_flow_endpoints(&page),
        [
            (8, 14381, Some("192.0.2.4")),
            (8, 14383, Some("192.0.2.4")),
            (8, 14387, Some("192.0.2.4")),
        ]
    );
    assert!(page
        .flows
        .iter()
        .all(|flow| flow.latency_nanoseconds == 10_000_000));
}

#[test]
fn issue_55_mxwani4_receiver_flows_use_ten_word_bitmaps() {
    for (fixture_name, address, routing) in [
        (
            "mxwani4_first_receiver_flows",
            "192.0.2.5",
            vec![vec![1], vec![2], vec![3], vec![4]],
        ),
        (
            "mxwani4_second_receiver_flows",
            "192.0.2.6",
            vec![vec![1], vec![2], vec![], vec![]],
        ),
    ] {
        let page = parse_receiver_flow_page(&payload(&["issue_55", fixture_name])).unwrap();

        assert_eq!(page.reported_flow_count, 1, "{fixture_name}");
        assert_eq!(
            page.flows[0].receiver_bitmap_word_count, 10,
            "{fixture_name}"
        );
        assert_eq!(receiver_flow_routing(&page), [routing], "{fixture_name}");
        assert_eq!(
            receiver_flow_endpoints(&page),
            [(8, 14337, Some(address))],
            "{fixture_name}"
        );
        assert_eq!(
            page.flows[0].latency_nanoseconds, 5_000_000,
            "{fixture_name}"
        );
    }
}

#[test]
fn issue_55_via_control_receiver_flows_still_parse() {
    let page = parse_receiver_flow_page(&payload(&["issue_55", "via_receiver_flows"])).unwrap();

    assert_eq!(
        receiver_flow_routing(&page),
        [vec![vec![2, 4], vec![5], vec![], vec![]]]
    );
    assert!(page
        .flows
        .iter()
        .all(|flow| flow.latency_nanoseconds == 10_000_000));
}
