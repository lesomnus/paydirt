#[cfg(test)]
mod operator_tests {
    use super::*;
    #[test]
    fn uuid_options_distinguish_trace_bytes_from_identifiers() {
        let schema: Value = serde_json::from_str(fleet::transport::ADMISSION_SCHEMA).unwrap();
        let bytes = STANDARD.encode([0xfe; 16]);
        let mut audit = json!({"id":bytes,"traceId":bytes});
        translate(&mut audit, "app.Audit", &schema, false, None).unwrap();
        assert_eq!(audit["traceId"], bytes);
        assert_eq!(audit["id"], "fefefefe-fefe-fefe-fefe-fefefefefefe");
        translate(&mut audit, "app.Audit", &schema, true, None).unwrap();
        assert_eq!(audit["id"], bytes);
    }
}
