from __future__ import annotations

import importlib
from typing import Any, cast

import torch
from transformers.modeling_outputs import BaseModelOutput
from transformers.models.wav2vec2_conformer import modeling_wav2vec2_conformer as conformer

ATTENTION = "eager"


def install(attention: str = ATTENTION) -> None:
    muq_model = importlib.import_module("muq.muq.models.muq_model")
    easydict = importlib.import_module("easydict").EasyDict

    def conformer_config(values: Any = None, **extra: Any) -> Any:
        config = easydict(values, **extra)
        if "_attn_implementation" not in config:
            config._attn_implementation = attention
        return config

    vars(muq_model)["EasyDict"] = conformer_config
    vars(conformer)["Wav2Vec2ConformerEncoder"] = RecordingEncoder


class RecordingEncoder(conformer.Wav2Vec2ConformerEncoder):
    def forward(
        self,
        hidden_states: torch.Tensor,
        attention_mask: torch.Tensor | None = None,
        output_hidden_states: bool = False,
        **kwargs: Any,
    ) -> BaseModelOutput:
        entries: list[torch.Tensor] = []
        handles = [
            layer.register_forward_pre_hook(lambda _, args: entries.append(args[0]))
            for layer in self.layers
        ]
        try:
            out = super().forward(hidden_states, attention_mask=attention_mask, **kwargs)
        finally:
            for handle in handles:
                handle.remove()
        last = out.last_hidden_state
        recorded = cast(tuple[torch.FloatTensor, ...], (*entries, last))
        return BaseModelOutput(last_hidden_state=last, hidden_states=recorded)
