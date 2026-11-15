# Generates icon.svg: a fibrous navy quill writing a doodle on a white tile.
#
#     python3 feather.py                      # in app/icons: writes icon.svg
#     cargo tauri icon icon-1024.png -o out   # from a 1024 px rendering of it;
#                                             # copy 32x32.png, 128x128.png, icon.png,
#                                             # icon.icns and icon.ico back here, then
#                                             # touch ../tauri.conf.json so the build
#                                             # picks up the new icon.
import math

B, C1, C2, T = (338, 768), (420, 600), (600, 330), (904, 122)  # the shaft, base to tip

def bez(t):
    u = 1 - t
    return tuple(u**3 * B[i] + 3 * u * u * t * C1[i] + 3 * u * t * t * C2[i] + t**3 * T[i] for i in range(2))

def tangent(t):
    u = 1 - t
    d = [3 * u * u * (C1[i] - B[i]) + 6 * u * t * (C2[i] - C1[i]) + 3 * t * t * (T[i] - C2[i]) for i in range(2)]
    n = math.hypot(*d)
    return d[0] / n, d[1] / n

T0 = 0.11  # where the plume starts

def width(t, side):
    """Fiber length: broad on the upper side, widest past the middle, rounded at the tip."""
    if t < T0:
        return 0
    s = (t - T0) / (1 - T0)
    w = 230 * math.sin(math.pi * min(1, s ** 0.75)) ** 0.8 + 12 * (1 - s)
    return w if side > 0 else w * 0.66

def fiber(t, side, frac=(0, 1)):
    """A fiber leaving the shaft at t, sweeping out and up toward the tip."""
    p = bez(t)
    dx, dy = tangent(t)
    nx, ny = (dy, -dx) if side > 0 else (-dy, dx)
    w = width(t, side)
    a = math.radians(58)
    end = (p[0] + w * (math.cos(a) * nx + math.sin(a) * dx), p[1] + w * (math.cos(a) * ny + math.sin(a) * dy))
    ctrl = (p[0] + w * 0.5 * nx + w * 0.25 * dx, p[1] + w * 0.5 * ny + w * 0.25 * dy)
    def q(f):
        u = 1 - f
        return (u * u * p[0] + 2 * u * f * ctrl[0] + f * f * end[0], u * u * p[1] + 2 * u * f * ctrl[1] + f * f * end[1])
    a0, a1 = frac
    pts = [q(a0 + (a1 - a0) * k / 8) for k in range(9)]
    return pts

def path(pts):
    return 'M' + ' L'.join(f'{x:.1f} {y:.1f}' for x, y in pts)

steps = [T0 + (1 - T0) * k / 70 for k in range(71)]
# The plume's outline: the fiber ends, up one side and back down the other.
upper = [fiber(t, +1)[-1] for t in steps]
lower = [fiber(t, -1)[-1] for t in reversed(steps)]
outline = [bez(T0)] + upper + [T] + lower
plume = path(outline) + ' Z'

fibers = ''.join(f'<path d="{path(fiber(t, s))}"/>' for t in steps[::2] for s in (+1, -1) if width(t, s) > 14)
# Splits: gaps opening from the edge inward, between groups of fibers.
splits = ''.join(f'<path d="{path(fiber(t, s, (0.22, 1.08)))}"/>' for t, s in [(0.36, 1), (0.58, 1), (0.79, 1), (0.52, -1)])
# Loose down near the base.
down = ''
for i, t in enumerate([0.05, 0.08, 0.11]):
    for s in (+1, -1):
        p = bez(t); dx, dy = tangent(t); nx, ny = (dy, -dx) if s > 0 else (-dy, dx)
        L = (110 + 30 * i) * (1 if s > 0 else 0.75)
        c = (p[0] + L * 0.7 * nx + L * 0.2 * dx, p[1] + L * 0.7 * ny + L * 0.2 * dy)
        e = (p[0] + L * (0.5 * nx + 0.9 * dx), p[1] + L * (0.5 * ny + 0.9 * dy))
        down += f'<path d="M{p[0]:.1f} {p[1]:.1f} Q {c[0]:.1f} {c[1]:.1f} {e[0]:.1f} {e[1]:.1f}"/>'

svg = f'''<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1024 1024" width="1024" height="1024">
  <defs>
    <linearGradient id="nib" x1="0" y1="0" x2="1" y2="0">
      <stop offset="0" stop-color="#f4f7f9"/>
      <stop offset="1" stop-color="#93a5b3"/>
    </linearGradient>
  </defs>
  <!-- the tile: plain white -->
  <rect x="40" y="40" width="944" height="944" rx="210" fill="#ffffff"/>
  <rect x="40" y="40" width="944" height="944" rx="210" fill="none" stroke="#d9dde5" stroke-width="10"/>
  <!-- the doodle: ink flowing from the nib, with a loop -->
  <path d="M292 838 C 360 920, 520 930, 560 850 C 596 776, 500 742, 478 812 C 456 886, 620 922, 790 846"
        fill="none" stroke="#1b2747" stroke-width="54" stroke-linecap="round" stroke-linejoin="round"/>
  <g fill="#1b2747"><circle cx="842" cy="804" r="20"/><circle cx="880" cy="770" r="11"/><circle cx="214" cy="760" r="14"/></g>
  <!-- the feather, made of fibers sweeping from the shaft out and up -->
  <g fill="none" stroke="#1d2b52" stroke-width="8" stroke-linecap="round">{down}</g>
  <path d="{plume}" fill="#1d2b52" stroke="#1d2b52" stroke-width="6" stroke-linejoin="round"/>
  <g fill="none" stroke="#3b5788" stroke-width="5" stroke-linecap="round">{fibers}</g>
  <g fill="none" stroke="#ffffff" stroke-width="15" stroke-linecap="round">{splits}</g>
  <!-- shaft -->
  <path d="M{B[0]} {B[1]} C {C1[0]} {C1[1]}, {C2[0]} {C2[1]}, {T[0]} {T[1]}" fill="none" stroke="#5a92b8" stroke-width="13" stroke-linecap="round"/>
  <!-- silver nib -->
  <g transform="translate({B[0]} {B[1]}) rotate(26)">
    <path d="M-20 -6 L 20 -6 L 8 70 L 0 92 L -8 70 Z" fill="url(#nib)" stroke="#6f8494" stroke-width="5" stroke-linejoin="round"/>
    <path d="M0 30 L 0 84" stroke="#6f8494" stroke-width="4" stroke-linecap="round"/>
  </g>
</svg>
'''
import os
open(os.path.join(os.path.dirname(os.path.abspath(__file__)), 'icon.svg'), 'w').write(svg)
