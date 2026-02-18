/**
 * Test script for the new analyze_video_frames builtin tool
 *
 * This tool extracts frames from a video file and analyzes them using a vision-capable LLM.
 *
 * Features:
 * - Extracts frames at specified intervals (default: 5 seconds)
 * - Analyzes each frame with a vision model
 * - Can identify objects, read text/graphs, describe scenes
 * - Useful for:
 *   - Understanding video content without watching
 *   - Extracting information from presentation videos
 *   - Analyzing graphs/charts in educational videos
 *   - Summarizing video tutorials
 *
 * Usage Example:
 *
 * User: "Can you analyze this video and tell me what's in it?"
 * (with video file attached)
 *
 * The LLM will automatically use the builtin_analyze_video_frames tool with:
 * {
 *   "file_path": "/full/path/to/video.mp4",
 *   "model_id": "qwen2-vl-7b",  // or any loaded vision-capable model
 *   "frame_interval": 5.0,       // extract frame every 5 seconds
 *   "max_frames": 10,            // limit to 10 frames
 *   "analysis_prompt": "Describe what you see in detail..."  // optional custom prompt
 * }
 *
 * The tool will return:
 * {
 *   "success": true,
 *   "file_path": "/path/to/video.mp4",
 *   "frames_analyzed": 10,
 *   "frame_interval_seconds": 5.0,
 *   "frame_analyses": [
 *     {
 *       "frame_number": 1,
 *       "timestamp_seconds": 0.0,
 *       "analysis": "The frame shows a presentation slide with a bar graph..."
 *     },
 *     {
 *       "frame_number": 2,
 *       "timestamp_seconds": 5.0,
 *       "analysis": "This frame shows a person explaining concepts..."
 *     },
 *     // ... more frames
 *   ]
 * }
 *
 * Requirements:
 * - ffmpeg must be installed (same as for transcribe_file tool)
 * - A vision-capable model must be loaded (e.g., Qwen2-VL, LLaVA, etc.)
 *
 * Use Cases:
 * 1. Analyze presentation videos:
 *    "What charts and graphs are shown in this presentation video?"
 *
 * 2. Understand tutorial content:
 *    "Summarize the steps shown in this tutorial video"
 *
 * 3. Extract text from videos:
 *    "What text appears on screen in this video?"
 *
 * 4. Identify objects/scenes:
 *    "What locations and objects are visible in this video?"
 *
 * 5. Analyze educational content:
 *    "What mathematical formulas or diagrams are shown?"
 */

console.log("Video analysis tool ready!");
console.log("This tool can extract and analyze frames from video files.");
console.log("It requires:");
console.log("  1. ffmpeg installed");
console.log("  2. A vision-capable model loaded (e.g., Qwen2-VL)");
console.log("  3. A video file to analyze");
