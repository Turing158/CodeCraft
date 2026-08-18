import {
  questionAnswerOptions,
  type ClaudeAnswerOption,
  type ClaudeQuestion,
  type ClaudeQuestionRequest,
} from "./claude-sessions";

export interface QuestionDraft {
  selectedAnswerIds: Set<string>;
  extraText: string;
  collapsed: boolean;
}

export type QuestionAction = "previous" | "next" | "submit";

export interface LocalQuestionAnswer {
  question: string;
  selectedOptionLabels: string[];
  extraText: string | null;
}

export interface LocalQuestionSubmission {
  requestId: string;
  answers: LocalQuestionAnswer[];
  contentSignature?: string;
}

export interface PendingQuestionResolution {
  request: ClaudeQuestionRequest | undefined;
  dismissedRequestId: string | undefined;
}

export function createQuestionDraft(): QuestionDraft {
  return {
    selectedAnswerIds: new Set<string>(),
    extraText: "",
    collapsed: false,
  };
}

export function createQuestionDrafts(
  request: ClaudeQuestionRequest,
): QuestionDraft[] {
  return request.questions.map(() => createQuestionDraft());
}

export function isQuestionAnswered(draft: QuestionDraft): boolean {
  return draft.selectedAnswerIds.size > 0;
}

export function updateQuestionSelection(
  question: ClaudeQuestion,
  draft: QuestionDraft,
  option: ClaudeAnswerOption,
): QuestionDraft {
  const selectedAnswerIds = new Set(draft.selectedAnswerIds);
  const wasSelected = selectedAnswerIds.has(option.id);

  if (option.kind === "chat" || !question.multiSelect) {
    selectedAnswerIds.clear();
  } else {
    for (const id of selectedAnswerIds) {
      if (id.endsWith(":chat")) selectedAnswerIds.delete(id);
    }
  }

  if (!wasSelected || !question.multiSelect || option.kind === "chat") {
    selectedAnswerIds.add(option.id);
  } else {
    selectedAnswerIds.delete(option.id);
  }

  return {
    ...draft,
    selectedAnswerIds,
  };
}

export function questionActions(
  currentIndex: number,
  totalQuestions: number,
  draft: QuestionDraft,
): QuestionAction[] {
  const actions: QuestionAction[] = [];

  if (currentIndex > 0) actions.push("previous");
  if (isQuestionAnswered(draft)) {
    actions.push(currentIndex < totalQuestions - 1 ? "next" : "submit");
  }

  return actions;
}

export function createLocalQuestionSubmission(
  request: ClaudeQuestionRequest,
  drafts: QuestionDraft[],
): LocalQuestionSubmission {
  return {
    requestId: request.id,
    answers: request.questions.map((question, questionIndex) => {
      const draft = drafts[questionIndex] ?? createQuestionDraft();
      const selectedOptions = questionAnswerOptions(
        request.id,
        question,
        questionIndex,
      ).filter((option) => draft.selectedAnswerIds.has(option.id));

      return {
        question: question.question,
        selectedOptionLabels: selectedOptions.map((option) => option.label),
        extraText: selectedOptions.some((option) => option.kind === "other")
          ? draft.extraText
          : null,
      };
    }),
  };
}

export function questionRequestContentSignature(
  request: ClaudeQuestionRequest,
): string {
  return JSON.stringify(request.questions);
}

export function resolvePendingQuestionRequest(
  requests: Array<ClaudeQuestionRequest | null>,
  dismissedRequestId: string | undefined,
  submittedContentSignature: string | undefined = undefined,
): PendingQuestionResolution {
  const pendingRequests = requests.filter(
    (request): request is ClaudeQuestionRequest => request !== null,
  );

  if (!dismissedRequestId) {
    return {
      request: pendingRequests[0],
      dismissedRequestId: undefined,
    };
  }

  const isSubmittedRequest = (request: ClaudeQuestionRequest) =>
    request.id === dismissedRequestId ||
    (submittedContentSignature !== undefined &&
      questionRequestContentSignature(request) === submittedContentSignature);

  const replacementRequest = pendingRequests.find(
    (request) => !isSubmittedRequest(request),
  );
  if (replacementRequest) {
    return {
      request: replacementRequest,
      dismissedRequestId: undefined,
    };
  }

  if (pendingRequests.some(isSubmittedRequest)) {
    return {
      request: undefined,
      dismissedRequestId,
    };
  }

  return {
    request: undefined,
    dismissedRequestId: undefined,
  };
}
