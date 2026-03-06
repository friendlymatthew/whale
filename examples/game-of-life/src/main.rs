use anyhow::{bail, Context, Result};
use gabagool::{CompiledInterpreter, Value};
use minifb::{Key, Window, WindowOptions};
use std::time::{Duration, Instant};

const CELL_PX: usize = 8;
// const SNAPSHOT_FILE: &str = "game-of-life.gabagool";

const PALETTE_NAMES: &[&str] = &["amber", "green", "blue", "pink", "white"];

fn call_i32(interpreter: &mut CompiledInterpreter, name: &str) -> Result<i32> {
    let result = interpreter.invoke(name, vec![])?;

    let Value::I32(v) = result.into_completed()?[0] else {
        bail!("expected i32");
    };

    Ok(v)
}

fn read_framebuf(interpreter: &CompiledInterpreter, ptr: usize, len: usize) -> &[u32] {
    let data = &interpreter.store().memories[0].data;
    let bytes = &data[ptr..ptr + len * 4];

    // safety: wasm memory is page aligned
    unsafe { std::slice::from_raw_parts(bytes.as_ptr() as *const u32, len) }
}

// todo: re-enable once CompiledInterpreter supports snapshots
// fn save_snapshot(interpreter: &CompiledInterpreter, path: &str) -> Result<()> {
//     let bytes = interpreter
//         .snapshot()
//         .context("failed to create snapshot")?;
//
//     std::fs::write(path, &bytes).context("failed to write snapshot file")
// }
//
// fn fork_snapshot(interpreter: &CompiledInterpreter, fork_count: u32) -> Result<()> {
//     let snapshot_bytes = interpreter
//         .snapshot()
//         .context("failed to create snapshot")?;
//
//     let id = std::time::SystemTime::now()
//         .duration_since(std::time::UNIX_EPOCH)?
//         .as_millis();
//
//     let path = format!("game-of-life-{id}.gabagool");
//     std::fs::write(&path, &snapshot_bytes)?;
//     let exe = std::env::current_exe().context("failed to get current exe")?;
//
//     Command::new(exe)
//         .arg("--restore")
//         .arg(&path)
//         .arg("--offset")
//         .arg(fork_count.to_string())
//         .spawn()
//         .context("failed to fork")?;
//
//     Ok(())
// }

fn main() -> Result<()> {
    let wasm_bytes = include_bytes!("../wasm/game.wasm");
    let mut interpreter = CompiledInterpreter::new(wasm_bytes).context("failed to load wasm")?;
    interpreter.invoke("init", vec![])?;

    // todo: re-enable once CompiledInterpreter supports snapshots
    // let args: Vec<String> = std::env::args().collect();
    // let restore_path = args
    //     .iter()
    //     .position(|a| a == "--restore")
    //     .map(|i| args.get(i + 1).map(|s| s.as_str()).unwrap_or(SNAPSHOT_FILE));
    let win_offset: i32 = 0;

    let grid_size = call_i32(&mut interpreter, "get_grid_size")? as usize;
    let win_size = grid_size * CELL_PX;
    let framebuf_ptr = call_i32(&mut interpreter, "get_framebuf_ptr")? as usize;

    let shift = (win_offset as isize) * 40;
    let mut window = Window::new(
        "Game of Life",
        win_size,
        win_size,
        WindowOptions {
            topmost: false,
            ..WindowOptions::default()
        },
    )
    .context("failed to create window")?;
    window.set_position(100 + shift, 100 + shift);
    window.set_target_fps(60);

    let mut paused = false;
    let mut palette: usize = 0;
    // let mut fork_count: u32 = 0;
    let mut last_tick = Instant::now();
    let tick_interval = Duration::from_millis(100);

    let mut prev_s = false;
    // let mut prev_f = false;
    let mut prev_space = false;

    interpreter.invoke("render", vec![])?;
    let pixel_count = win_size * win_size;

    while window.is_open() && !window.is_key_down(Key::Escape) {
        let cur_s = window.is_key_down(Key::S);
        // let cur_f = window.is_key_down(Key::F);
        let cur_space = window.is_key_down(Key::Space);

        if cur_s && !prev_s {
            // todo: re-enable snapshot saving once CompiledInterpreter supports it
            // if let Err(e) = save_snapshot(&interpreter, SNAPSHOT_FILE) {
            //     eprintln!("snapshot error: {e}");
            // }
            palette = (palette + 1) % PALETTE_NAMES.len();
            interpreter.invoke("set_palette", vec![Value::I32(palette as i32)])?;
            println!("Palette: {}", PALETTE_NAMES[palette]);
            interpreter.invoke("render", vec![])?;
        }

        // todo: re-enable fork once CompiledInterpreter supports snapshots
        // if cur_f && !prev_f {
        //     fork_count += 1;
        //     if let Err(e) = fork_snapshot(&interpreter, fork_count) {
        //         eprintln!("fork error: {e}");
        //     }
        // }

        if cur_space && !prev_space {
            paused = !paused;
            println!("{}", if paused { "Paused" } else { "Resumed" });
        }

        prev_s = cur_s;
        // prev_f = cur_f;
        prev_space = cur_space;

        if !paused && last_tick.elapsed() >= tick_interval {
            interpreter.invoke("tick", vec![])?;
            interpreter.invoke("render", vec![])?;
            last_tick = Instant::now();

            let gen = call_i32(&mut interpreter, "get_generation")?;
            window.set_title(&format!("Game of Life | generation: {gen}"));
        }

        let framebuf = read_framebuf(&interpreter, framebuf_ptr, pixel_count);
        window.update_with_buffer(framebuf, win_size, win_size)?;
    }

    Ok(())
}
