# Slither Neuroevolution

A modernized neuroevolution simulation based on the classic Slither.io game mechanics. Snakes evolve neural networks (MLP + GRU) to survive, eat food, and compete against each other.

## Features

- **Neuroevolution**: Snakes use specific brain architectures including Dense layers and Gated Recurrent Units (GRU) memory.
- **Modern Tech Stack**: Built with Vite and ES Modules.
- **Visual Polish**:
  - Particle system for boosts and death effects.
  - Glow effects and dynamic lighting.
  - Responsive, high-performance rendering.
- **Real-time Visualization**:
  - **Brain Visualizer**: See the active neural network of the focused snake.
  - **Fitness Chart**: Track population performance over generations.

## Prerequisites

You need **Node.js** installed on your computer.

## Installation

1.  Clone this repository (if you haven't already).
2.  Open a terminal in the project folder.
3.  Install dependencies:

    ```bash
    npm install
    ```

## How to Run

Because this project uses ES Modules, **you cannot simply open `index.html` in a file browser**. You must run it through a local development server.

### Development (Recommended)

To start the simulation with hot-reloading:

```bash
npm run dev
```

Click the local URL shown in the terminal (usually `http://localhost:5173`) to open the simulation.

### Production Build

To build the project for deployment:

```bash
npm run build
```

The output will be in the `dist/` folder. You can preview the production build locally with:

```bash
npm run preview
```

## Controls

- **V**: Toggle Camera Mode (Overview / Follow Focused Snake).
- **UI Controls**: Use the panel on the left to adjust population size, simulation speed, and neural network topology.
- **Reset**: Applying changes to core settings (like layer count) will reset the simulation.
