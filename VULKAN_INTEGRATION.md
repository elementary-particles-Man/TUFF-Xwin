# Vulkan Integration Design

## 目的
シミュレーションモードを廃止し、実際に Vulkan API を使用して Compute Shader を実行する。

## 実装内容
1. **初期化 (Initialization)**
   - `ash` を使用して `Instance` を作成。
   - 計算用途（Compute Queue）を持つ `PhysicalDevice` を選択。
   - `Logical Device` を作成。

2. **パイプライン (Pipeline)**
   - 入力の `u32` 配列の各要素に `1` を加算する Compute Shader を実装。
   - `DescriptorSetLayout`, `PipelineLayout`, `ComputePipeline` を構築。

3. **実行 (Execution)**
   - `submit_batch` 時に `Storage Buffer` を作成し、データを転送。
   - コマンドバッファを記録し、Queue に投入。
   - `Fence` を使用して完了を追跡し、`poll_completion` で状態を返す。

4. **後片付け (Cleanup)**
   - `Drop` トレイトによりリソースを解放。

## シェーダー (GLSL)
```glsl
#version 450
layout(local_size_x = 64) in;
layout(std430, binding = 0) buffer Data {
    uint values[];
};
void main() {
    uint idx = gl_GlobalInvocationID.x;
    values[idx] += 1;
}
```
これを SPIR-V にコンパイルしたバイト列を `lib.rs` に埋め込む。
