use burn_import::onnx::ModelGen;

fn main() {
    // Re-run this build script if the onnx-tests directory changes.
    println!("cargo:rerun-if-changed=tests");

    // Add onnx models.
    ModelGen::new()
        .input("tests/add/add.onnx")
        .input("tests/avg_pool2d/avg_pool2d.onnx")
        .input("tests/concat/concat.onnx")
        .input("tests/conv1d/conv1d.onnx")
        .input("tests/conv2d/conv2d.onnx")
        .input("tests/div/div.onnx")
        .input("tests/dropout/dropout_opset16.onnx")
        .input("tests/dropout/dropout_opset7.onnx")
        .input("tests/flatten/flatten.onnx")
        .input("tests/global_avr_pool/global_avr_pool.onnx")
        .input("tests/log_softmax/log_softmax.onnx")
        .input("tests/maxpool2d/maxpool2d.onnx")
        .input("tests/mul/mul.onnx")
        .input("tests/reshape/reshape.onnx")
        .input("tests/softmax/softmax.onnx")
        .input("tests/sub/sub.onnx")
        .out_dir("model/")
        .run_from_script();

    // panic!("Purposefully failing build to output logs.");
}
