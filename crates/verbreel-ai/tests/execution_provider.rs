use verbreel_ai::{
    ExecutionProvider, ORT_AUTO_PROMOTE_ORDER_V1, ORT_AUTO_PROMOTE_ORDER_V1_IDS,
    ort_auto_promote_order_v1,
};

#[test]
fn execution_provider_as_str_is_canonical() {
    assert_eq!(ExecutionProvider::Cuda.as_str(), "cuda");
    assert_eq!(ExecutionProvider::TensorRt.as_str(), "tensorrt");
    assert_eq!(ExecutionProvider::DirectMl.as_str(), "directml");
    assert_eq!(ExecutionProvider::CoreMl.as_str(), "coreml");
    assert_eq!(ExecutionProvider::Cpu.as_str(), "cpu");
}

#[test]
fn ort_auto_promote_order_v1_is_fixed() {
    assert_eq!(
        ORT_AUTO_PROMOTE_ORDER_V1,
        [
            ExecutionProvider::Cuda,
            ExecutionProvider::TensorRt,
            ExecutionProvider::DirectMl,
            ExecutionProvider::CoreMl,
            ExecutionProvider::Cpu,
        ]
    );
}

#[test]
fn ort_auto_promote_order_v1_helper_matches_constant() {
    assert_eq!(ort_auto_promote_order_v1(), ORT_AUTO_PROMOTE_ORDER_V1);
}

#[test]
fn ort_auto_promote_order_v1_ids_match_constant_order() {
    assert_eq!(
        ORT_AUTO_PROMOTE_ORDER_V1_IDS,
        ["cuda", "tensorrt", "directml", "coreml", "cpu"]
    );

    let from_enum: Vec<&'static str> = ORT_AUTO_PROMOTE_ORDER_V1
        .iter()
        .map(ExecutionProvider::as_str)
        .collect();
    assert_eq!(from_enum, ORT_AUTO_PROMOTE_ORDER_V1_IDS);
}
