#define SIZE 64
#define TOTAL (SIZE * SIZE)
#define CELL_PX 8
#define WIN_SIZE (SIZE * CELL_PX)

static unsigned char grid_a[TOTAL];
static unsigned char grid_b[TOTAL];
static int current_is_a = 1;
static int generation = 0;

static unsigned int framebuf[WIN_SIZE * WIN_SIZE];

#define NUM_PALETTES 5
static unsigned int palette_alive[NUM_PALETTES] = {
    0x00FFCC00,
    0x0000FF66,
    0x0044AAFF,
    0x00FF44AA,
    0x00FFFFFF,
};
static unsigned int palette_dead[NUM_PALETTES] = {
    0x00111111,
    0x00111111,
    0x00111111,
    0x00111111,
    0x00111111,
};
static int current_palette = 0;

static int mod(int x, int m) {
    int r = x % m;
    return r < 0 ? r + m : r;
}

static int count_neighbors(unsigned char *grid, int x, int y) {
    int count = 0;
    for (int dy = -1; dy <= 1; dy++) {
        for (int dx = -1; dx <= 1; dx++) {
            if (dx == 0 && dy == 0) continue;
            int nx = mod(x + dx, SIZE);
            int ny = mod(y + dy, SIZE);
            count += grid[ny * SIZE + nx];
        }
    }
    return count;
}

void set_cell(int x, int y, int alive) {
    unsigned char *grid = current_is_a ? grid_a : grid_b;
    grid[y * SIZE + x] = alive ? 1 : 0;
}

void init(void) {
    for (int i = 0; i < TOTAL; i++) {
        grid_a[i] = 0;
        grid_b[i] = 0;
    }
    generation = 0;
    current_is_a = 1;

    int cx = 32, cy = 32;
    set_cell(cx + 1, cy + 0, 1);
    set_cell(cx + 2, cy + 0, 1);
    set_cell(cx + 0, cy + 1, 1);
    set_cell(cx + 1, cy + 1, 1);
    set_cell(cx + 1, cy + 2, 1);

    set_cell(5, 4, 1);
    set_cell(6, 5, 1);
    set_cell(4, 6, 1);
    set_cell(5, 6, 1);
    set_cell(6, 6, 1);
}

void tick(void) {
    unsigned char *src = current_is_a ? grid_a : grid_b;
    unsigned char *dst = current_is_a ? grid_b : grid_a;

    for (int y = 0; y < SIZE; y++) {
        for (int x = 0; x < SIZE; x++) {
            int idx = y * SIZE + x;
            int n = count_neighbors(src, x, y);
            int alive = src[idx];
            if (alive) {
                dst[idx] = (n == 2 || n == 3) ? 1 : 0;
            } else {
                dst[idx] = (n == 3) ? 1 : 0;
            }
        }
    }

    current_is_a = !current_is_a;
    generation++;
}

int get_grid_ptr(void) {
    return current_is_a ? (int)(unsigned long)grid_a : (int)(unsigned long)grid_b;
}

int get_grid_size(void) {
    return SIZE;
}

int get_generation(void) {
    return generation;
}

void render(void) {
    unsigned char *grid = current_is_a ? grid_a : grid_b;
    unsigned int alive = palette_alive[current_palette];
    unsigned int dead  = palette_dead[current_palette];

    for (int y = 0; y < SIZE; y++) {
        for (int x = 0; x < SIZE; x++) {
            unsigned int color = grid[y * SIZE + x] ? alive : dead;
            for (int py = 0; py < CELL_PX; py++) {
                for (int px = 0; px < CELL_PX; px++) {
                    framebuf[(y * CELL_PX + py) * WIN_SIZE + (x * CELL_PX + px)] = color;
                }
            }
        }
    }
}

int get_framebuf_ptr(void) {
    return (int)(unsigned long)framebuf;
}

void set_palette(int idx) {
    current_palette = idx % NUM_PALETTES;
}

int get_palette(void) {
    return current_palette;
}
