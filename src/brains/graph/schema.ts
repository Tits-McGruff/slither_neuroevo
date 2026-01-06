/** Supported node types for graph-based brains. */
export type GraphNodeType = 'Input' | 'Dense' | 'MLP' | 'GRU' | 'LSTM' | 'RRU' | 'Concat' | 'Split';

/** Base node shape shared by all graph nodes. */
export interface GraphNodeBase {
  id: string;
  type: GraphNodeType;
}

/** Input node definition. */
export interface InputNodeSpec extends GraphNodeBase {
  type: 'Input';
  outputSize: number;
}

/** Dense node definition. */
export interface DenseNodeSpec extends GraphNodeBase {
  type: 'Dense';
  inputSize: number;
  outputSize: number;
}

/** MLP node definition. */
export interface MlpNodeSpec extends GraphNodeBase {
  type: 'MLP';
  inputSize: number;
  outputSize: number;
  hiddenSizes?: number[];
}

/** GRU node definition. */
export interface GruNodeSpec extends GraphNodeBase {
  type: 'GRU';
  inputSize: number;
  hiddenSize: number;
}

/** LSTM node definition. */
export interface LstmNodeSpec extends GraphNodeBase {
  type: 'LSTM';
  inputSize: number;
  hiddenSize: number;
}

/** RRU node definition. */
export interface RruNodeSpec extends GraphNodeBase {
  type: 'RRU';
  inputSize: number;
  hiddenSize: number;
}

/** Concat node definition for merging inputs. */
export interface ConcatNodeSpec extends GraphNodeBase {
  type: 'Concat';
}

/** Split node definition for splitting outputs. */
export interface SplitNodeSpec extends GraphNodeBase {
  type: 'Split';
  outputSizes: number[];
}

/** Union of all supported graph node specs. */
export type GraphNodeSpec =
  | InputNodeSpec
  | DenseNodeSpec
  | MlpNodeSpec
  | GruNodeSpec
  | LstmNodeSpec
  | RruNodeSpec
  | ConcatNodeSpec
  | SplitNodeSpec;

/** Directed edge between two node ports. */
export interface GraphEdge {
  from: string;
  to: string;
  fromPort?: number;
  toPort?: number;
}

/** Output reference to a node and optional port. */
export interface GraphOutputRef {
  nodeId: string;
  port?: number;
}

/** Full graph specification used to build brains. */
export interface GraphSpec {
  type: 'graph';
  nodes: GraphNodeSpec[];
  edges: GraphEdge[];
  outputs: GraphOutputRef[];
  outputSize: number;
}
