from PIL import Image, ImageEnhance, ImageFilter
import numpy as np
import json
import cv2
from sklearn.cluster import KMeans

# Expanded ASCII character set for finer gradation
chars = ["@", "%", "#", "W", "&", "8", "B", "M", "K", "X", "D", "Q", "H", "A", "O", "Z", "U", "*", "+", "=", "-", ":", ".", " "]

def resize_frame(image: Image, new_width: int = 150) -> Image:
    """Resizes the image for higher resolution ASCII output."""
    width, height = image.size
    aspect_ratio = height / width
    new_height = int(aspect_ratio * new_width * 0.55)
    return image.resize((new_width, new_height))

def adaptive_thresholding(image: Image) -> Image:
    """Applies adaptive thresholding to manage noise and preserve details."""
    np_image = np.array(image)
    thresholded_image = cv2.adaptiveThreshold(np_image, 255, cv2.ADAPTIVE_THRESH_GAUSSIAN_C, cv2.THRESH_BINARY, 11, 2)
    return Image.fromarray(thresholded_image)

def bilateral_filtering(image: Image) -> Image:
    """Applies bilateral filtering for noise reduction while preserving edges."""
    np_image = np.array(image)
    filtered_image = cv2.bilateralFilter(np_image, d=9, sigmaColor=75, sigmaSpace=75)
    return Image.fromarray(filtered_image)

def clustering_character_mapping(image: Image, num_clusters: int = 8) -> np.ndarray:
    """Clusters brightness levels and maps clusters to ASCII characters."""
    np_image = np.array(image).reshape(-1, 1)
    kmeans = KMeans(n_clusters=num_clusters)
    kmeans.fit(np_image)
    
    # Map each pixel's cluster to a unique ASCII character
    cluster_centers = np.sort(kmeans.cluster_centers_.flatten())
    cluster_mapping = {i: chars[int(i * len(chars) / num_clusters)] for i in range(num_clusters)}
    
    clustered_image = np.vectorize(cluster_mapping.get)(kmeans.labels_).reshape(image.size[::-1])
    return clustered_image

def perceptual_brightness_mapping(image: Image) -> str:
    """Maps pixel brightness to ASCII characters with finer gradation."""
    np_image = np.array(image)
    ascii_art = ""
    
    # Using expanded character set for smoother transitions
    perceptual_chars = ["@", "%", "#", "W", "&", "8", "B", "M", "K", "X", "D", "Q", "H", "A", "O", "Z", "U", "*", "+", "=", "-", ":", ".", " "]
    
    for row in np_image:
        for pixel in row:
            # Scale the brightness to the character set range
            index = int((pixel / 255) * (len(perceptual_chars) - 1))
            ascii_art += perceptual_chars[index]
        ascii_art += "\n"
    return ascii_art

def convert_gif_to_ascii_json(gif_path: str, output_path: str, width: int = 150):
    """Main function to convert GIF frames to ASCII art and save as JSON."""
    with Image.open(gif_path) as gif:
        frames = []
        frame_duration = gif.info['duration']
        
        frame_idx = 0
        while True:
            try:
                gif.seek(frame_idx)
                frame = gif.convert("L")
                
                # Resize, adaptive threshold, and apply noise reduction
                resized_image = resize_frame(frame, width)
                adaptive_thresh_image = adaptive_thresholding(resized_image)
                bilateral_image = bilateral_filtering(adaptive_thresh_image)
                
                # Clustered brightness mapping to ensure smoother transitions
                clustered_image = clustering_character_mapping(bilateral_image)
                
                # Final perceptual brightness mapping to generate ASCII art
                ascii_frame = perceptual_brightness_mapping(bilateral_image)
                
                frames.append(ascii_frame.strip())
                
                print(f"Processed frame {frame_idx + 1}")
                frame_idx += 1
            except EOFError:
                break

    # Save frames to JSON with frame duration metadata
    with open(output_path, "w") as f:
        json.dump({"frame_duration": frame_duration, "frames": frames}, f)

    print(f"Saved frames to {output_path}")

# Usage
convert_gif_to_ascii_json("input.gif", "ascii_gif_frames.json", width=80)
