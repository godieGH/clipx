/**
 * @file manage-tauri.mjs
 * @description CLI task runner for Tauri application maintenance and build pipeline orchestration.
 * 
 * High-level capabilities:
 *  1. Clean UI distribution output (`dist/`).
 *  2. Sync app branding assets (copies `src-tauri/icons/128x128.png` -> `src/public/`)
 *     to ensure single-source-of-truth icon usage across frontend and desktop app.
 *  3. Purge Cargo workspace artifacts (`cargo clean`).
 * 
 * @usage
 *  node manage-tauri.mjs [flag|command]
 * 
 * @example
 *  node manage-tauri.mjs --clean
 *  node manage-tauri.mjs --sync-icons
 *  node manage-tauri.mjs --cargo-clean
 *  node manage-tauri.mjs --all
 */

import fs from 'node:fs';
import path from 'node:path';
import { exec } from 'node:child_process';
import { promisify } from 'node:util';

const execAsync = promisify(exec);
const ROOT_DIR = path.resolve(".");

// --- Task Definitions ---

/**
 * Task 1: Clean frontend dist folder
 */
async function cleanDist() {
  const distPath = path.join(ROOT_DIR, 'dist');
  console.log('🧹 Cleaning dist directory...');
  
  if (fs.existsSync(distPath)) {
    fs.rmSync(distPath, { recursive: true, force: true });
    console.log('  ✔ dist/ removed.');
  } else {
    console.log('  ℹ dist/ directory does not exist. Skipping.');
  }
}

/**
 * Task 2: Sync icons from src-tauri to frontend public asset folder
 */
async function syncIcons() {
  const sourceIcon = path.join(ROOT_DIR, 'src-tauri', 'icons', '128x128.png');
  const targetDir = path.join(ROOT_DIR, 'public');
  const targetIcon = path.join(targetDir, 'clipx-icon.png');

  console.log('🎨 Syncing Tauri icon to Vite public directory...');

  if (!fs.existsSync(sourceIcon)) {
    console.error(`  ✖ Error: Source icon not found at ${sourceIcon}`);
    process.exit(1);
  }

  // Ensure target directory exists
  if (!fs.existsSync(targetDir)) {
    fs.mkdirSync(targetDir, { recursive: true });
  }

  fs.copyFileSync(sourceIcon, targetIcon);
  console.log(`  ✔ Copied 128x128.png to ${targetIcon}`);
}

/**
 * Task 3: Clean cargo workspace artifacts
 */
async function cleanCargo() {
  console.log('🦀 Cleaning Cargo build artifacts...');
  try {
    const { stdout } = await execAsync('cargo clean --package=clipx-app');
    if (stdout.trim()) console.log(stdout);
    console.log('  ✔ Package clipx-app cleaned successfully.');
  } catch (error) {
    console.error('  ✖ Failed to clean cargo package:', error.message);
    process.exit(1);
  }
}

async function cleanCargoAll() {
  console.log('🦀 Cleaning Cargo build artifacts...');
  try {
    const { stdout } = await execAsync('cargo clean');
    if (stdout.trim()) console.log(stdout);
    console.log('  ✔ Cargo workspace cleaned successfully.');
  } catch (error) {
    console.error('  ✖ Failed to clean cargo workspace:', error.message);
    process.exit(1);
  }
}

/**
 * Displays CLI usage helper
 */
function showHelp() {
  console.log(`
Usage: node manage-tauri.mjs [options]

Options:
  --clean-dist    Remove the dist/ directory
  --sync-icons    Copy icon from src-tauri/icons/ to src/public/
  --cargo-clean   Run cargo clean on the Rust workspace
  --all           Run all tasks in sequence
  --help, -h      Display this help menu
  `);
}

// --- CLI Argument Handler ---

async function run() {
  const args = process.argv.slice(2);

  if (args.length === 0 || args.includes('--help') || args.includes('-h')) {
    showHelp();
    return;
  }

  const runAll = args.includes('--all');

  try {
    if (runAll || args.includes('--clean-dist')) {
      await cleanDist();
    }

    if (runAll || args.includes('--sync-icons')) {
      await syncIcons();
    }

    if (args.includes('--cargo-clean')) {
      await cleanCargo();
    }

    if (runAll || args.includes('--cargo-clean-all')) {
      await cleanCargoAll();
    }

    console.log('\n✨ Task execution completed!');
  } catch (err) {
    console.error('\n✖ Pipeline failed:', err.message);
    process.exit(1);
  }
}

run();
