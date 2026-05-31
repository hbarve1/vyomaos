"use client";

import { useRef } from "react";
import { Canvas, useFrame } from "@react-three/fiber";
import { Float, MeshDistortMaterial } from "@react-three/drei";
import type { Mesh } from "three";

function AnimatedSphere() {
  const meshRef = useRef<Mesh>(null);

  useFrame((_, delta) => {
    if (meshRef.current) {
      meshRef.current.rotation.y += delta * 0.3;
      meshRef.current.rotation.x += delta * 0.1;
    }
  });

  return (
    <Float speed={2} rotationIntensity={0.4} floatIntensity={1}>
      <mesh ref={meshRef} scale={1.8}>
        <icosahedronGeometry args={[1, 4]} />
        <MeshDistortMaterial
          color="#58A6FF"
          roughness={0.2}
          metalness={0.8}
          distort={0.25}
          speed={2}
          wireframe
        />
      </mesh>
    </Float>
  );
}

function Particles() {
  const groupRef = useRef<Mesh>(null);

  useFrame((_, delta) => {
    if (groupRef.current) {
      groupRef.current.rotation.y += delta * 0.05;
    }
  });

  return (
    <mesh ref={groupRef}>
      {Array.from({ length: 80 }).map((_, i) => {
        const theta = Math.random() * Math.PI * 2;
        const phi = Math.acos(2 * Math.random() - 1);
        const r = 3 + Math.random() * 1.5;
        const x = r * Math.sin(phi) * Math.cos(theta);
        const y = r * Math.sin(phi) * Math.sin(theta);
        const z = r * Math.cos(phi);
        return (
          <mesh key={i} position={[x, y, z]}>
            <sphereGeometry args={[0.02, 6, 6]} />
            <meshBasicMaterial color="#58A6FF" transparent opacity={0.5} />
          </mesh>
        );
      })}
    </mesh>
  );
}

export default function Scene3D() {
  return (
    <div className="h-[320px] w-full sm:h-[400px]">
      <Canvas camera={{ position: [0, 0, 6], fov: 45 }}>
        <ambientLight intensity={0.3} />
        <directionalLight position={[5, 5, 5]} intensity={0.8} />
        <pointLight position={[-5, -5, -5]} intensity={0.4} color="#3FB950" />
        <AnimatedSphere />
        <Particles />
      </Canvas>
    </div>
  );
}
