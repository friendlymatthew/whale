# Game of life

<img src="../../demo.gif" width="80%" alt="Game of Life demo">

This demo runs Conway's game of life inside `gabagool`. The game logic is a small C program compiled to wasm. The host loads the wasm, calls `tick` and `render` each frame, and reads the framebuffer directly out of linear memory to draw to a window.

The cool part is the snapshotting. At any point you can serialize the interpreter state to a `.gabagool` file and restore it later exactly where you left off. You can also fork the game and it will snapshot the running instance and spawn a new process from that snapshot.

# Usage

Press `F` to fork a new window from the the current state<br>
Press `Spacebar` to pause/resume execution

```sh
# note: all these commands are assuming you're in /examples/game-of-life/

# to run the game
cargo r

# if you want to rebuild the wasm
# let's say you edited game.c
brew install llvm
rustup target add wasm32-unknown-unknown
cd wasm && make
```
