import type { FreshRunGraphWire, FreshRunGraphNodeWire, FreshRunGraphEdgeWire, FreshRunGraphOutputWire } from '../../../contracts/types.ts';

/** Shared raw graph shapes; compiler validation stays in each consumer. */
export type GraphNodeSpec = FreshRunGraphNodeWire;
export type GraphNodeType = GraphNodeSpec['type'];
export interface GraphNodeBase { id: string; type: GraphNodeType }
export type InputNodeSpec = Extract<GraphNodeSpec, {type: 'Input'}>;
export type DenseNodeSpec = Extract<GraphNodeSpec, {type: 'Dense'}>;
export type MlpNodeSpec = Extract<GraphNodeSpec, {type: 'MLP'}>;
export type GruNodeSpec = Extract<GraphNodeSpec, {type: 'GRU'}>;
export type LstmNodeSpec = Extract<GraphNodeSpec, {type: 'LSTM'}>;
export type RruNodeSpec = Extract<GraphNodeSpec, {type: 'RRU'}>;
export type ConcatNodeSpec = Extract<GraphNodeSpec, {type: 'Concat'}>;
export type SplitNodeSpec = Extract<GraphNodeSpec, {type: 'Split'}>;
/** UI graphs omit absent ports; native wire admission also accepts null. */
export type GraphEdge = Omit<FreshRunGraphEdgeWire, 'fromPort' | 'toPort'> & {fromPort?: number; toPort?: number};
export type GraphOutputRef = Omit<FreshRunGraphOutputWire, 'port'> & {port?: number};
export type GraphSpec = Omit<FreshRunGraphWire, 'graph_type' | 'type' | 'edges' | 'outputs'> & {
  type: 'graph';
  edges: GraphEdge[];
  outputs: GraphOutputRef[];
};
