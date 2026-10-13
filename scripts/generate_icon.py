# /// script
# requires-python = ">=3.10"
# dependencies = ["pillow==12.3.0", "resvg-py==0.5.0"]
# ///
"""Regenerate the Windows icon: uv run scripts/generate_icon.py."""

from io import BytesIO
from pathlib import Path
import struct

from PIL import Image
import resvg_py


ASSETS = Path(__file__).resolve().parents[1] / "assets"
SIZES = (16, 20, 24, 28, 32, 40, 48, 56, 64, 72, 96, 128, 192, 256)


def render(size: int, *, small: bool = False) -> Image.Image:
    source = ASSETS / ("keep-window-small.svg" if small else "keep-window.svg")
    data = resvg_py.svg_to_bytes(
        svg_path=str(source), width=size * 4, height=size * 4
    )
    return Image.open(BytesIO(data)).convert("RGBA").resize(
        (size, size), Image.Resampling.LANCZOS
    )


def icon_frame(image: Image.Image) -> bytes:
    size = image.width
    if size == 256:
        output = BytesIO()
        image.save(output, format="PNG")
        return output.getvalue()

    # Small frames use 32-bit BGRA DIBs with a bottom-up transparency mask.
    pixels = image.transpose(Image.Transpose.FLIP_TOP_BOTTOM)
    mask_stride = ((size + 31) // 32) * 4
    mask = bytearray(mask_stride * size)
    alpha = pixels.getchannel("A").tobytes()
    for y in range(size):
        for x in range(size):
            if alpha[y * size + x] == 0:
                mask[y * mask_stride + x // 8] |= 0x80 >> (x % 8)
    color = pixels.tobytes("raw", "BGRA")
    header = struct.pack(
        "<IiiHHIIiiII", 40, size, size * 2, 1, 32, 0, len(color), 0, 0, 0, 0
    )
    return header + color + mask


def main() -> None:
    frames = [icon_frame(render(size, small=size < 32)) for size in SIZES]
    offset = 6 + len(SIZES) * 16
    entries = []
    for size, frame in zip(SIZES, frames):
        entries.append(
            struct.pack("<BBBBHHII", size % 256, size % 256, 0, 0, 1, 32, len(frame), offset)
        )
        offset += len(frame)
    (ASSETS / "keep-window.ico").write_bytes(
        struct.pack("<HHH", 0, 1, len(SIZES)) + b"".join(entries) + b"".join(frames)
    )
    render(1024).save(ASSETS / "keep-window.png")
    with Image.open(ASSETS / "keep-window.ico") as icon:
        assert icon.ico.sizes() == {(size, size) for size in SIZES}
        for size in SIZES:
            assert icon.ico.getimage((size, size)).size == (size, size)
    print(f"Generated keep-window.ico ({len(SIZES)} sizes) and keep-window.png (1024 px)")


if __name__ == "__main__":
    main()
