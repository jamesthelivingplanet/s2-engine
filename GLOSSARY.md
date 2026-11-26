# S2 Engine

Runs language models on this machine, one at a time.

## Language

**Model**:
The weights on disk that can be run.
_Avoid_: checkpoint, LLM

**Runtime**:
A named build of a model server that can run models.
_Avoid_: binary, engine

**Profile**:
A named, runnable entry combining a model, a runtime and settings. Several profiles may share one model.
_Avoid_: model (when the runnable entry is meant), preset

**Hook**:
A command of the user's own that runs before a profile's model starts, or after it ends.
_Avoid_: script, trigger

**Source**:
The place a model can be downloaded from.
_Avoid_: URL, origin
