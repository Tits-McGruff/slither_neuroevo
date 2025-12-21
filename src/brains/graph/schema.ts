export type GraphNodeType = 'Input' | 'Dense' | 'MLP' | 'GRU' | 'LSTM' | 'RRU' | 'Concat' | 'Split';

export interface GraphNodeBase {
  id: string;
  type: GraphNodeType;
}

export interface InputNodeSpec extends GraphNodeBase {
  type: 'Input';
  outputSize: number;
}

export interface DenseNodeSpec extends GraphNodeBase {
  type: 'Dense';
  inputSize: number;
  outputSize: number;
}

export interface MlpNodeSpec extends GraphNodeBase {
  type: 'MLP';
  inputSize: number;
  outputSize: number;
  hiddenSizes?: number[];
}

export interface GruNodeSpec extends GraphNodeBase {
  type: 'GRU';
  inputSize: number;
  hiddenSize: number;
}

export interface LstmNodeSpec extends GraphNodeBase {
  type: 'LSTM';
  inputSize: number;
  hiddenSize: number;
}

export interface RruNodeSpec extends GraphNodeBase {
  type: 'RRU';
  inputSize: number;
  hiddenSize: number;
}

export interface ConcatNodeSpec extends GraphNodeBase {
  type: 'Concat';
}

export interface SplitNodeSpec extends GraphNodeBase {
  type: 'Split';
  outputSizes: number[];
}

export type GraphNodeSpec =
  | InputNodeSpec
  | DenseNodeSpec
  | MlpNodeSpec
  | GruNodeSpec
  | LstmNodeSpec
  | RruNodeSpec
  | ConcatNodeSpec
  | SplitNodeSpec;

export interface GraphEdge {
  from: string;
  to: string;
  fromPort?: number;
  toPort?: number;
}

export interface GraphOutputRef {
  nodeId: string;
  port?: number;
}

export interface GraphSpec {
  type: 'graph';
  nodes: GraphNodeSpec[];
  edges: GraphEdge[];
  outputs: GraphOutputRef[];
  outputSize: number;
}
