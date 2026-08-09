# Attribution

Canonical binary artifacts are derived from third-party tokenizer and algorithmic-complexity data. 

## Tokenizer sources

The finalized lexicon was built from one-token vocabulary strings from these representative tokenizer files:

- OpenAI `o200k_base`: [public encoding file](https://openaipublic.blob.core.windows.net/encodings/o200k_base.tiktoken).
- [Qwen/Qwen3.5-0.8B-Base](https://huggingface.co/Qwen/Qwen3.5-0.8B-Base), `tokenizer.json`.
- [deepseek-ai/DeepSeek-V3.2](https://huggingface.co/deepseek-ai/DeepSeek-V3.2), `tokenizer.json`.
- [mistralai/Ministral-3-3B-Base-2512](https://huggingface.co/mistralai/Ministral-3-3B-Base-2512), `tekken.json`.
- [moonshotai/Kimi-K2.5](https://huggingface.co/moonshotai/Kimi-K2.5), `tiktoken.model`.
- [zai-org/GLM-5.2](https://huggingface.co/zai-org/GLM-5.2), `tokenizer.json`.
- [google/gemma-3-1b-pt](https://huggingface.co/google/gemma-3-1b-pt), `tokenizer.json`.

Tokenizer names, model names, and upstream licenses belong to their respective owners. This repository does not redistribute model weights or the upstream tokenizer files; the canonical lexicon contains only ordinary strings selected from their vocabulary behavior.

For the Gemma proxy, Google’s terms are available at <https://ai.google.dev/gemma/terms>. Gemma is provided under and subject to the Gemma Terms of Use found at that URL.

## CTM source

The offline CTM-B9-D12 prior was derived from the direct joint CTM data in [pybdm](https://pypi.org/project/pybdm/), version 0.1.0, under its MIT license. The bundled prior was shaped as unordered 6+6 pairs over the nine-symbol B9 alphabet and used only to select the static prefix points.
