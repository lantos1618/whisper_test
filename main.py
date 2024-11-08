from PIL import Image, ImageEnhance
import numpy as np
import json

def resize_frame(image: Image, new_width: int = 100) -> Image:
    width, height = image.size
    aspect_ratio = height / width
    new_height = int(aspect_ratio * new_width * 0.55)
    resized_image = image.resize((new_width, new_height))
    return resized_image

def enhance_contrast(image: Image, factor: float = 3.0) -> Image:
    grayscale_image = image.convert("L")  # Ensure image is grayscale
    enhancer = ImageEnhance.Contrast(grayscale_image)
    return enhancer.enhance(factor)

def apply_gamma_correction(image: Image, gamma: float = 0.5) -> Image:
    inv_gamma = 1.0 / gamma
    lut = [min(int((i / 255.0) ** inv_gamma * 255), 255) for i in range(256)]
    return image.point(lut)

# Mapping Strategies

def linear_bias_mapping(pixel: int, num_chars: int) -> int:
    """Linear mapping with a slight bias towards mid-range values."""
    return min(int(pixel * (num_chars - 1) / 255), num_chars - 1)

def logarithmic_mapping(pixel: int, num_chars: int) -> int:
    """Logarithmic mapping to preserve highlights and shadows."""
    normalized = np.log1p(pixel) / np.log1p(255)
    return min(int(normalized * (num_chars - 1)), num_chars - 1)

def piecewise_mapping(pixel: int, num_chars: int) -> int:
    """Piecewise mapping: custom thresholds for shadows, mid-tones, and highlights."""
    if pixel < 85:       # Shadows
        return int(pixel / 85 * (num_chars // 3))
    elif pixel < 170:    # Mid-tones
        return int((pixel - 85) / 85 * (num_chars // 3)) + num_chars // 3
    else:                # Highlights
        return int((pixel - 170) / 85 * (num_chars // 3)) + 2 * num_chars // 3

def frame_to_ascii(image: Image, mapping_func) -> str:
    # Extended character set for greater detail
    chars = "█▓▒░@%#*+=-:. "
    pixels = np.array(image)
    ascii_str = ""
    
    for row in pixels:
        for pixel in row:
            # Use selected mapping function
            mapped_index = mapping_func(int(pixel), len(chars))
            ascii_str += chars[mapped_index]
        ascii_str += "\n"
    return ascii_str

def convert_gif_to_ascii_json(gif_path: str, output_path: str, width: int = 100, contrast: float = 2.0, gamma: float = 0.5, mapping_func=linear_bias_mapping):
    with Image.open(gif_path) as gif:
        frames = []
        frame_duration = gif.info['duration']  # Duration in milliseconds
        
        frame_idx = 0
        while True:
            try:
                gif.seek(frame_idx)  # Move to the frame
                frame = gif.copy()
                
                # Resize, enhance contrast, and apply gamma correction
                resized_image = resize_frame(frame, width)
                enhanced_image = enhance_contrast(resized_image, contrast)
                gamma_corrected_image = apply_gamma_correction(enhanced_image, gamma)
                
                # Convert to ASCII using the selected mapping function
                ascii_frame = frame_to_ascii(gamma_corrected_image, mapping_func)
                frames.append(ascii_frame.strip())
                
                print(f"Processed frame {frame_idx + 1}")
                frame_idx += 1
            except EOFError:
                break  # End of frames

    # Save frames to JSON with frame duration metadata
    with open(output_path, "w") as f:
        json.dump({"frame_duration": frame_duration, "frames": frames}, f)

    print(f"Saved frames to {output_path}")

# Usage - Testing different mapping strategies
# Use linear_bias_mapping, logarithmic_mapping, or piecewise_mapping as mapping_func
convert_gif_to_ascii_json("input.gif", "ascii_gif_frames.json", width=100, contrast=2.0, gamma=0.5, mapping_func=logarithmic_mapping)
